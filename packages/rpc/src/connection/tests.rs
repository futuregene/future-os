use super::*;

fn ready(home: &Path) -> AgentReadinessPayload {
    AgentReadinessPayload {
        version: "1.2.3-agent-dirty".into(),
        agent_instance_id: "agent-a".into(),
        protocol_version: AGENT_PROTOCOL_VERSION,
        data_root_id: data_root_id(home).unwrap(),
        storage_schema_version: AGENT_STORAGE_SCHEMA_VERSION,
    }
}

#[test]
fn compatibility_checks_release_protocol_schema_and_canonical_data_root() {
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let expected = CompatibilityRequirements {
        version: "1.2.3-desktop-dirty".into(),
        expected_home: Some(home.path().to_owned()),
    };
    let valid = ready(home.path());
    validate_readiness(&valid, &expected).unwrap();
    for invalid in [
        AgentReadinessPayload {
            version: "1.2.4".into(),
            ..valid.clone()
        },
        AgentReadinessPayload {
            protocol_version: 0,
            ..valid.clone()
        },
        AgentReadinessPayload {
            storage_schema_version: 0,
            ..valid.clone()
        },
        AgentReadinessPayload {
            data_root_id: data_root_id(other.path()).unwrap(),
            ..valid.clone()
        },
        AgentReadinessPayload {
            agent_instance_id: String::new(),
            ..valid.clone()
        },
    ] {
        assert!(validate_readiness(&invalid, &expected).is_err());
    }
}

#[test]
fn checking_a_stale_lock_does_not_change_its_contents_or_metadata() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join("agent")).unwrap();
    let path = home.path().join("agent").join("agent-instance.lock");
    let metadata = home.path().join("agent").join("agent-instance.json");
    std::fs::write(&path, b"preserve lock bytes").unwrap();
    std::fs::write(&metadata, b"preserve metadata").unwrap();
    let mut owner = fd_lock::RwLock::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap(),
    );
    let guard = owner.try_write().unwrap();
    assert!(check_instance_lock(home.path()).is_err());
    drop(guard);
    check_instance_lock(home.path()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"preserve lock bytes");
    assert_eq!(std::fs::read(metadata).unwrap(), b"preserve metadata");
    assert!(!home.path().join("agent").join("agent.db").exists());
}

#[tokio::test]
async fn unavailable_explicit_tcp_is_never_a_local_start_permission() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(listener);
    let result = select_agent(
        &address,
        "1.2.3",
        Duration::from_millis(300),
        Duration::from_millis(300),
    )
    .await;
    assert!(matches!(result, Err(ConnectionError::StartupBlocked(_))));
}

#[derive(Clone)]
struct MockAgent {
    info: std::sync::Arc<std::sync::Mutex<AgentReadinessPayload>>,
    mutations: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    handshake_failure: u8,
}

#[tonic::async_trait]
impl proto::future_agent_server::FutureAgent for MockAgent {
    async fn execute_command(
        &self,
        request: tonic::Request<proto::RpcCommand>,
    ) -> Result<tonic::Response<proto::RpcResponse>, tonic::Status> {
        let command = request.into_inner();
        if command.r#type == "get_agent_readiness" {
            match self.handshake_failure {
                1 => return Err(tonic::Status::unavailable("readiness unavailable")),
                2 => {
                    return Ok(tonic::Response::new(proto::RpcResponse {
                        success: false,
                        error: "readiness rejected".into(),
                        ..Default::default()
                    }))
                }
                3 => std::future::pending::<()>().await,
                _ => {}
            }
        }
        let data = if command.r#type == "get_agent_readiness" {
            serde_json::to_string(&*self.info.lock().unwrap()).unwrap()
        } else {
            self.mutations
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            "{}".to_owned()
        };
        Ok(tonic::Response::new(proto::RpcResponse {
            id: command.id,
            r#type: "response".into(),
            command: command.r#type,
            success: true,
            data,
            ..Default::default()
        }))
    }
    type StreamEventsStream = std::pin::Pin<
        Box<dyn tokio_stream::Stream<Item = Result<proto::StreamEvent, tonic::Status>> + Send>,
    >;
    async fn stream_events(
        &self,
        _: tonic::Request<proto::StreamRequest>,
    ) -> Result<tonic::Response<Self::StreamEventsStream>, tonic::Status> {
        self.mutations
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(tonic::Response::new(Box::pin(tokio_stream::empty())))
    }
}

async fn serve_mock(
    listener: tokio::net::TcpListener,
    mock: MockAgent,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(proto::future_agent_server::FutureAgentServer::new(mock))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = stopped.await;
                },
            )
            .await
            .unwrap();
    });
    (stop, task)
}

fn mock_agent() -> MockAgent {
    MockAgent {
        info: std::sync::Arc::new(std::sync::Mutex::new(AgentReadinessPayload {
            version: "1.2.3".into(),
            agent_instance_id: "agent-a".into(),
            protocol_version: AGENT_PROTOCOL_VERSION,
            data_root_id: "0".repeat(64),
            storage_schema_version: AGENT_STORAGE_SCHEMA_VERSION,
        })),
        mutations: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        handshake_failure: 0,
    }
}

#[tokio::test]
async fn failed_or_timed_out_readiness_never_dispatches_business_or_authorizes_start() {
    for handshake_failure in [1, 2, 3] {
        let mut mock = mock_agent();
        mock.handshake_failure = handshake_failure;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let (_stop, task) = serve_mock(listener, mock.clone()).await;
        let result = select_agent(
            &address,
            "1.2.3",
            Duration::from_secs(1),
            Duration::from_millis(100),
        )
        .await;
        assert!(matches!(
            result,
            Err(ConnectionError::Unverified(_) | ConnectionError::Incompatible { .. })
        ));
        assert_eq!(mock.mutations.load(std::sync::atomic::Ordering::Acquire), 0);
        // Deliberately hung readiness is fault injection; drop its server task.
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn incompatible_and_changed_instances_receive_no_business_requests_or_streams() {
    let mock = mock_agent();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (stop, task) = serve_mock(listener, mock.clone()).await;
    let mut client = connect_verified(
        &address,
        "1.2.3",
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    mock.info.lock().unwrap().agent_instance_id = "agent-b".into();
    assert!(client
        .execute_command(proto::RpcCommand {
            r#type: "set_auth".into(),
            ..Default::default()
        })
        .await
        .is_err());
    assert!(client
        .stream_events(proto::StreamRequest {
            global_events: true,
            ..Default::default()
        })
        .await
        .is_err());
    let mut client = connect_verified(
        &address,
        "1.2.3",
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    mock.info.lock().unwrap().data_root_id = "1".repeat(64);
    assert!(client
        .execute_command(proto::RpcCommand {
            r#type: "new_session".into(),
            ..Default::default()
        })
        .await
        .is_err());
    mock.info.lock().unwrap().protocol_version = 0;
    assert!(matches!(
        select_agent(
            &address,
            "1.2.3",
            Duration::from_secs(1),
            Duration::from_secs(1)
        )
        .await,
        Err(ConnectionError::Incompatible { .. })
    ));
    assert_eq!(mock.mutations.load(std::sync::atomic::Ordering::Acquire), 0);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cached_physical_channel_cannot_reconnect_to_a_legacy_replacement() {
    let first = mock_agent();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, task) = serve_mock(listener, first).await;
    let mut client = connect_verified(
        &address.to_string(),
        "1.2.3",
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    // The replacement deliberately ignores expected-instance metadata. Safety
    // comes from refusing transparent reconnect, before it sees any operation.
    let legacy = mock_agent();
    legacy.info.lock().unwrap().protocol_version = 0;
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    let (stop, task) = serve_mock(listener, legacy.clone()).await;
    assert!(client
        .execute_command(proto::RpcCommand {
            r#type: "delete_sessions".into(),
            ..Default::default()
        })
        .await
        .is_err());
    assert!(matches!(
        connect_verified(
            &address.to_string(),
            "1.2.3",
            Duration::from_secs(1),
            Duration::from_secs(1)
        )
        .await,
        Err(ConnectionError::Incompatible { .. })
    ));
    assert_eq!(
        legacy.mutations.load(std::sync::atomic::Ordering::Acquire),
        0
    );
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
}
