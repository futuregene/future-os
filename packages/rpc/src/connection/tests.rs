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

/// A held lock must be reported with the owner identity from the metadata the
/// running Agent publishes — the TUI's preflight says what `future agent`
/// itself would say, instead of a generic "occupied" complaint.
#[test]
fn occupied_lock_reports_the_owner_identity_from_metadata() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join("agent")).unwrap();
    let path = home.path().join("agent").join("agent-instance.lock");
    let metadata = home.path().join("agent").join("agent-instance.json");
    std::fs::write(&path, b"").unwrap();
    std::fs::write(
        &metadata,
        serde_json::json!({
            "pid": 4242,
            "host": "login03",
            "executable": "/home/u/.local/bin/future",
        })
        .to_string(),
    )
    .unwrap();
    let mut owner = fd_lock::RwLock::new(
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap(),
    );
    let guard = owner.try_write().unwrap();

    let error = check_instance_lock(home.path()).expect_err("occupied");
    // A held lock is a yield signal, not a fault: supervision waits for the owner
    // instead of reporting a failure, so this is `InstanceBusy` by type.
    assert!(
        error.is_instance_busy(),
        "an occupied lock must classify as InstanceBusy, got: {error}"
    );
    let message = match error {
        ConnectionError::InstanceBusy(reason) => reason,
        other => panic!("expected InstanceBusy, got: {other}"),
    };
    assert!(message.contains("already running"), "{message}");
    assert!(message.contains("pid 4242"), "{message}");
    assert!(
        message.contains("held by:") || message.contains("host login03"),
        "missing owner identity: {message}"
    );
    // The metadata read is best-effort and must not disturb the owner.
    assert_eq!(
        std::fs::read(&metadata).unwrap(),
        serde_json::json!({
            "pid": 4242,
            "host": "login03",
            "executable": "/home/u/.local/bin/future",
        })
        .to_string()
        .into_bytes()
    );
    drop(guard);
    check_instance_lock(home.path()).unwrap();
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
pub(crate) struct MockAgent {
    info: std::sync::Arc<std::sync::Mutex<AgentReadinessPayload>>,
    mutations: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    handshake_failure: u8,
    /// How many readiness calls fail with a transient transport error before the
    /// Agent starts answering. Fault injection for the retry policy: a stalled
    /// or descheduled Agent looks exactly like this from the client side.
    transient_readiness_failures: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Pages answered to `get_event_content_range`, in order. Empty means the
    /// command fails, which is what a client that asked for more than the host
    /// published sees.
    contents: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<serde_json::Value>>>,
}

#[tonic::async_trait]
impl proto::future_agent_server::FutureAgent for MockAgent {
    async fn execute_command(
        &self,
        request: tonic::Request<proto::RpcCommand>,
    ) -> Result<tonic::Response<proto::RpcResponse>, tonic::Status> {
        let command = request.into_inner();
        if command.r#type == "get_agent_readiness"
            && self
                .transient_readiness_failures
                .try_update(
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                    |left| left.checked_sub(1),
                )
                .is_ok()
        {
            return Err(tonic::Status::unavailable(
                "readiness temporarily unavailable",
            ));
        }
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
                4 => {
                    return Ok(tonic::Response::new(proto::RpcResponse {
                        success: false,
                        error_code: "agent_stopping".into(),
                        error: "service stopping".into(),
                        ..Default::default()
                    }))
                }
                // A legacy Agent that predates the readiness RPC at all.
                5 => return Err(tonic::Status::unimplemented("no readiness")),
                // A success response whose body is not a readiness payload.
                6 => {
                    return Ok(tonic::Response::new(proto::RpcResponse {
                        success: true,
                        data: "not json".into(),
                        ..Default::default()
                    }))
                }
                _ => {}
            }
        }
        if command.r#type == "get_event_content_range" {
            let next = self.contents.lock().unwrap().pop_front();
            return Ok(tonic::Response::new(match next {
                Some(page) => proto::RpcResponse {
                    id: command.id,
                    r#type: "response".into(),
                    command: command.r#type,
                    success: true,
                    data: page.to_string(),
                    ..Default::default()
                },
                None => proto::RpcResponse {
                    id: command.id,
                    r#type: "response".into(),
                    command: command.r#type,
                    success: false,
                    error: "no content pages left".into(),
                    ..Default::default()
                },
            }));
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

pub(crate) async fn serve_mock(
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

pub(crate) fn mock_agent() -> MockAgent {
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
        transient_readiness_failures: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        contents: std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
    }
}

/// A mock that answers `get_event_content_range` with these pages, in order.
pub(crate) fn mock_agent_answering(pages: Vec<serde_json::Value>) -> MockAgent {
    MockAgent {
        contents: std::sync::Arc::new(std::sync::Mutex::new(pages.into())),
        ..mock_agent()
    }
}

#[tokio::test]
async fn a_transiently_unanswered_readiness_is_retried_rather_than_refused() {
    // One failed readiness, then the Agent answers: the case a descheduled or
    // storage-stalled Agent produces. A single attempt reports this as a refusal
    // that forbids starting locally, while the Agent was answering all along.
    let mut mock = mock_agent();
    mock.transient_readiness_failures = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(1));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (_stop, task) = serve_mock(listener, mock.clone()).await;
    let selection = crate::supervision::select_agent_with_retries(
        &address,
        "1.2.3",
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await;
    assert!(
        matches!(selection, Ok(AgentSelection::External(_))),
        "a retried readiness must attach, got: {:?}",
        selection.err()
    );
    // Readiness alone: a retry never turns into a business command.
    assert_eq!(mock.mutations.load(std::sync::atomic::Ordering::Acquire), 0);
    drop(_stop);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn readiness_retries_are_bounded_before_the_refusal_is_reported() {
    // Permanently unavailable readiness still ends as the same refusal, after a
    // bounded number of attempts: retrying must not hold an entry open.
    let mut mock = mock_agent();
    let remaining = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(99));
    mock.transient_readiness_failures = remaining.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (_stop, task) = serve_mock(listener, mock.clone()).await;
    let result = crate::supervision::select_agent_with_retries(
        &address,
        "1.2.3",
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await;
    assert!(
        matches!(result, Err(ConnectionError::Unverified(_))),
        "a permanently unavailable Agent must stay Unverified"
    );
    let attempts = 99 - remaining.load(std::sync::atomic::Ordering::Acquire);
    assert_eq!(
        attempts,
        crate::supervision::READINESS_PROBE_ATTEMPTS as usize
    );
    assert_eq!(mock.mutations.load(std::sync::atomic::Ordering::Acquire), 0);
    drop(_stop);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
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

#[tokio::test]
async fn stopping_agent_is_blocked_without_being_incompatible_or_authorizing_start() {
    let mut mock = mock_agent();
    mock.handshake_failure = 4;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (stop, task) = serve_mock(listener, mock.clone()).await;
    let result = select_agent(
        &address,
        "1.2.3",
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await;
    assert!(
        matches!(result, Err(ConnectionError::StartupBlocked(reason)) if reason.contains("stopping"))
    );
    assert_eq!(mock.mutations.load(std::sync::atomic::Ordering::Acquire), 0);
    let _ = stop.send(());
    task.await.unwrap();
}

#[test]
fn different_development_builds_share_only_the_explicit_release_protocol_schema_contract() {
    let home = tempfile::tempdir().unwrap();
    let expected = CompatibilityRequirements {
        version: "1.2.3-abc+dev".into(),
        expected_home: Some(home.path().into()),
    };
    let mut info = ready(home.path());
    info.version = "1.2.3-def+dev".into();
    validate_readiness(&info, &expected).unwrap();
    info.protocol_version += 1;
    assert!(matches!(
        validate_readiness(&info, &expected),
        Err(ConnectionError::Incompatible { .. })
    ));
    info.protocol_version = AGENT_PROTOCOL_VERSION;
    info.storage_schema_version += 1;
    assert!(validate_readiness(&info, &expected).is_err());
}

fn contract_metadata() -> Vec<(&'static str, String)> {
    vec![
        (EXPECTED_INSTANCE_HEADER, "agent-a".into()),
        (EXPECTED_DATA_ROOT_HEADER, "root-hash".into()),
        (CLIENT_VERSION_HEADER, "1.2.3".into()),
        (CLIENT_PROTOCOL_HEADER, AGENT_PROTOCOL_VERSION.to_string()),
        (
            CLIENT_SCHEMA_HEADER,
            AGENT_STORAGE_SCHEMA_VERSION.to_string(),
        ),
    ]
}

fn metadata(pairs: &[(&'static str, String)]) -> tonic::metadata::MetadataMap {
    let mut metadata = tonic::metadata::MetadataMap::new();
    for (header, value) in pairs {
        metadata.insert(*header, value.parse().expect("visible ASCII headers"));
    }
    metadata
}

/// Every business request carries the client contract, and the Agent's view of
/// instance, data root, protocol and schema must equal the verified readiness.
#[test]
fn a_business_request_without_a_matching_client_contract_is_refused() {
    let complete = contract_metadata();
    validate_request_metadata(&metadata(&complete), "1.2.3", "agent-a", "root-hash")
        .expect("the complete contract is accepted");

    // A development suffix is not a different client: only the release core
    // has to match.
    validate_request_metadata(&metadata(&complete), "1.2.3+dev", "agent-a", "root-hash")
        .expect("build metadata is ignored");

    let mut missing = complete.clone();
    missing.retain(|(header, _)| *header != CLIENT_SCHEMA_HEADER);
    let error = validate_request_metadata(&metadata(&missing), "1.2.3", "agent-a", "root-hash")
        .expect_err("an absent contract header must be refused");
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    assert!(
        error.message().contains("missing client contract"),
        "{}",
        error.message()
    );

    for (index, value, label) in [
        (0, "agent-b".into(), "another instance"),
        (1, "other-root".into(), "another data root"),
        (2, "1.2.4".into(), "a different client release"),
        (
            3,
            (AGENT_PROTOCOL_VERSION + 1).to_string(),
            "another protocol",
        ),
        (3, "not-a-number".into(), "an unparsable protocol"),
        (
            4,
            (AGENT_STORAGE_SCHEMA_VERSION + 1).to_string(),
            "another storage schema",
        ),
    ] {
        let mut changed = complete.clone();
        changed[index] = (changed[index].0, value);
        let error = validate_request_metadata(&metadata(&changed), "1.2.3", "agent-a", "root-hash")
            .expect_err(label);
        assert_eq!(error.code(), tonic::Code::FailedPrecondition, "{label}");
        assert!(
            error
                .message()
                .contains("reconnect with a matching FutureOS client"),
            "{label}: {}",
            error.message()
        );
    }
}

/// A header value that cannot be put on the wire must stop the request instead
/// of being dropped or silently coerced.
#[test]
fn a_contract_header_that_cannot_be_encoded_is_refused_before_dispatch() {
    let home = tempfile::tempdir().unwrap();
    let mut readiness = ready(home.path());

    readiness.agent_instance_id = "agent\ninjected".into();
    let error = bind_request(tonic::Request::new(()), &readiness, "1.2.3")
        .expect_err("a control character cannot be a header value");
    assert_eq!(error.code(), tonic::Code::FailedPrecondition);
    assert!(
        error
            .message()
            .contains("invalid client connection contract"),
        "{}",
        error.message()
    );

    readiness.agent_instance_id = "agent-a".into();
    let request =
        bind_request(tonic::Request::new(()), &readiness, "1.2.3").expect("a valid contract binds");
    let headers = request.metadata();
    assert_eq!(
        headers.get(EXPECTED_INSTANCE_HEADER).unwrap(),
        readiness.agent_instance_id.as_str()
    );
    assert_eq!(
        headers.get(EXPECTED_DATA_ROOT_HEADER).unwrap(),
        readiness.data_root_id.as_str()
    );
    assert_eq!(headers.get(CLIENT_VERSION_HEADER).unwrap(), "1.2.3");
    assert_eq!(
        headers
            .get(CLIENT_PROTOCOL_HEADER)
            .unwrap()
            .to_str()
            .unwrap(),
        AGENT_PROTOCOL_VERSION.to_string()
    );
    assert_eq!(
        headers.get(CLIENT_SCHEMA_HEADER).unwrap().to_str().unwrap(),
        AGENT_STORAGE_SCHEMA_VERSION.to_string()
    );
}

/// An operator has to be able to tell "the Agent refused me" from "I could not
/// start one", so the startup refusal renders with its reason attached.
#[test]
fn a_startup_refusal_renders_with_the_reason_it_was_refused_for() {
    assert_eq!(
        ConnectionError::StartupBlocked("port busy".into()).to_string(),
        "Cannot start a local Agent: port busy"
    );
}

/// A reachable, matching Agent is an external selection: the client keeps the
/// verified readiness (that is what every later request is bound to).
#[tokio::test]
async fn a_verified_agent_is_selected_as_an_external_client_carrying_its_readiness() {
    let mock = mock_agent();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (stop, task) = serve_mock(listener, mock.clone()).await;

    let selection = select_agent(
        &address,
        "1.2.3",
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .expect("a serving Agent is reachable");
    let AgentSelection::External(client) = selection else {
        panic!("a reachable Agent must not become a local-start permit");
    };
    assert_eq!(client.readiness().agent_instance_id, "agent-a");
    assert_eq!(client.readiness().version, "1.2.3");
    assert_eq!(mock.mutations.load(std::sync::atomic::Ordering::Acquire), 0);

    let _ = stop.send(());
    task.await.unwrap();
}

/// A local-start permit is only a permission for the configuration it was
/// issued under: a moved home or a changed endpoint invalidates it.
#[tokio::test]
async fn a_local_start_permit_is_valid_only_for_the_discovery_configuration_it_saw() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    let permit = LocalStartPermit {
        home: home.path().to_owned(),
        endpoint: PermitEndpoint::Local(transport::local_endpoint_label()),
    };
    assert_eq!(permit.future_home(), home.path());
    permit
        .recheck()
        .expect("an unchanged configuration is still valid");

    let moved = LocalStartPermit {
        home: home.path().join("elsewhere"),
        ..permit
    };
    let error = moved
        .recheck()
        .expect_err("a moved FutureOS home invalidates the permit");
    assert!(
        matches!(error, ConnectionError::StartupBlocked(ref reason) if reason.contains("discovery configuration changed")),
        "{error}"
    );

    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// A lock that exists but cannot be read is not "no Agent is running": it is a
/// refusal, because the caller cannot know whether another Agent owns the home.
#[tokio::test]
async fn an_unreadable_instance_lock_blocks_a_local_start() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    let permit = LocalStartPermit {
        home: home.path().to_owned(),
        endpoint: PermitEndpoint::Local(transport::local_endpoint_label()),
    };
    std::fs::create_dir_all(home.path().join("agent/agent-instance.lock")).unwrap();
    let error = permit
        .recheck()
        .expect_err("an unreadable lock must not authorize a start");
    assert!(
        matches!(error, ConnectionError::StartupBlocked(ref reason) if reason.contains("instance lock unavailable")),
        "{error}"
    );

    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// The whole point of the fallback: with no endpoint at all and no instance
/// lock, an unconfigured client is authorized to start a local Agent *for the
/// home it actually resolved* — not for a default or an arbitrary directory.
#[tokio::test]
// The environment lock is held across the awaits on purpose: the endpoint has to
// stay redirected for the whole resolution, and this test runs on a current-thread
// runtime so nothing else shares the thread.
#[allow(clippy::await_holding_lock)]
async fn an_absent_endpoint_authorizes_a_local_start_for_the_resolved_home() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    let previous_socket = std::env::var_os("FUTURE_AGENT_SOCKET");
    std::env::remove_var("FUTURE_AGENT_SOCKET");
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    let selection = select_agent(
        "auto",
        "1.2.3",
        Duration::from_millis(500),
        Duration::from_millis(500),
    )
    .await
    .expect("an absent endpoint with a free home is a local start");
    let AgentSelection::LocalStart(permit) = selection else {
        panic!("an absent endpoint cannot be an external client");
    };
    assert_eq!(
        permit.future_home(),
        home.path(),
        "the permit names the FutureOS home the client resolved"
    );
    assert!(
        !home.path().join("run").exists(),
        "preflight must not create the endpoint it is asking about"
    );

    match previous_socket {
        Some(value) => std::env::set_var("FUTURE_AGENT_SOCKET", value),
        None => std::env::remove_var("FUTURE_AGENT_SOCKET"),
    }
    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// A connect failure the client cannot classify as "the Agent is absent" (here:
/// Darwin answers ENOTSOCK for a regular file where a socket should be) must
/// never be read as permission to start one over it.
// macOS only: the fixture needs the kernel to REFUSE a connect to a regular file
// with ENOTSOCK, which is what makes the failure classifiable-by-nobody. Linux
// answers ECONNREFUSED for the same input, which the client reads as "the Agent is
// absent" and would legitimately authorize a start over - so the precondition
// cannot be built there. Recorded as a platform difference, not a waiver of the
// invariant: on Darwin the check below is what stops a start on top of a foreign
// occupant, and it is exercised on every macOS run.
#[cfg(target_os = "macos")]
#[tokio::test]
// The environment lock is held across the awaits on purpose: the endpoint has to
// stay redirected for the whole resolution, and this test runs on a current-thread
// runtime so nothing else shares the thread.
#[allow(clippy::await_holding_lock)]
async fn an_unclassifiable_local_endpoint_failure_never_authorizes_a_start() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    let previous_socket = std::env::var_os("FUTURE_AGENT_SOCKET");
    std::env::remove_var("FUTURE_AGENT_SOCKET");
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    let run = home.path().join("run");
    std::fs::create_dir_all(&run).unwrap();
    let endpoint = run.join("agent.sock");
    std::fs::write(&endpoint, "not a socket").unwrap();

    let error = select_agent(
        "auto",
        "1.2.3",
        Duration::from_millis(500),
        Duration::from_millis(500),
    )
    .await
    .err()
    .expect("an unclassifiable occupant must not authorize a local start");
    assert!(matches!(error, ConnectionError::Transport(_)), "{error}");
    assert_eq!(
        std::fs::read_to_string(&endpoint).unwrap(),
        "not a socket",
        "the occupant is left exactly as it was found"
    );

    match previous_socket {
        Some(value) => std::env::set_var("FUTURE_AGENT_SOCKET", value),
        None => std::env::remove_var("FUTURE_AGENT_SOCKET"),
    }
    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// The data-root identity is the anti-spoofing check: an Agent that reports
/// something that is not a 64-hex digest of a directory cannot be compared
/// against the client's own root, so it is refused rather than trusted.
#[test]
fn a_data_root_identity_that_is_not_a_digest_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let expected = CompatibilityRequirements {
        version: "1.2.3".into(),
        expected_home: None,
    };
    let valid = ready(home.path());
    for broken in ["", "abc", &"0".repeat(63), &"z".repeat(64)] {
        let info = AgentReadinessPayload {
            data_root_id: broken.to_string(),
            ..valid.clone()
        };
        let error = validate_readiness(&info, &expected)
            .expect_err("a data-root identity that is not a digest must be refused");
        assert!(
            matches!(error, ConnectionError::Incompatible { ref reason, .. } if reason.contains("data-root identity")),
            "{broken:?}: {error}"
        );
    }
    validate_readiness(&valid, &expected).expect("a 64-hex digest is accepted");
}

/// An expected home that cannot be hashed (it does not exist, so it cannot be
/// canonicalized) must refuse the readiness check instead of skipping it: an
/// unverifiable client is not a verified one.
#[test]
fn an_unhashable_expected_home_refuses_readiness_instead_of_skipping_it() {
    let home = tempfile::tempdir().unwrap();
    let missing = home.path().join("never-created");
    let expected = CompatibilityRequirements {
        version: "1.2.3".into(),
        expected_home: Some(missing.clone()),
    };
    let error = validate_readiness(&ready(home.path()), &expected)
        .expect_err("an unknown expected root must not be accepted");
    assert!(
        matches!(error, ConnectionError::Incompatible { ref reason, .. } if reason.contains("cannot establish the expected data root")),
        "{error}"
    );

    // The same readiness is fine when the client simply has no expected home.
    validate_readiness(
        &ready(home.path()),
        &CompatibilityRequirements {
            version: "1.2.3".into(),
            expected_home: None,
        },
    )
    .expect("no expectation, nothing to verify");
}

/// A legacy Agent that answers readiness with `unimplemented` is incompatible,
/// not "unverified": it will never implement readiness, so retrying is wrong.
#[tokio::test]
async fn an_agent_without_readiness_support_is_incompatible() {
    let mut mock = mock_agent();
    mock.handshake_failure = 5;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (stop, task) = serve_mock(listener, mock.clone()).await;
    let error = select_agent(
        &address,
        "1.2.3",
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .err()
    .expect("an agent without readiness must be refused");
    assert!(
        matches!(error, ConnectionError::Incompatible { ref reason, .. } if reason.contains("does not implement readiness")),
        "{error}"
    );
    assert_eq!(mock.mutations.load(std::sync::atomic::Ordering::Acquire), 0);
    let _ = stop.send(());
    task.await.unwrap();
}

/// A successful response whose body is not a readiness payload cannot be
/// verified either: the failure must say what was unreadable.
#[tokio::test]
async fn a_readiness_response_that_cannot_be_decoded_is_incompatible() {
    let mut mock = mock_agent();
    mock.handshake_failure = 6;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (stop, task) = serve_mock(listener, mock.clone()).await;
    let error = select_agent(
        &address,
        "1.2.3",
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .await
    .err()
    .expect("an undecodable readiness body must be refused");
    assert!(
        matches!(error, ConnectionError::Incompatible { ref reason, .. } if reason.contains("incomplete readiness response")),
        "{error}"
    );
    assert_eq!(mock.mutations.load(std::sync::atomic::Ordering::Acquire), 0);
    let _ = stop.send(());
    task.await.unwrap();
}

/// A host that was told to serve the Agent on an address may fill it: an
/// explicit TCP endpoint nothing answers on is a start permit, not the
/// "no local fallback" refusal an attach-only caller gets.
#[tokio::test]
// The environment lock is held across the awaits on purpose: the endpoint has to
// stay redirected for the whole resolution, and this test runs on a current-thread
// runtime so nothing else shares the thread.
#[allow(clippy::await_holding_lock)]
async fn a_serving_host_may_fill_its_own_loopback_endpoint() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    // A port nothing listens on: bind and immediately release it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let address = format!("127.0.0.1:{port}");

    let selection = select_agent_serving(
        &address,
        "1.2.3",
        Duration::from_millis(500),
        Duration::from_millis(500),
    )
    .await
    .expect("an absent loopback endpoint this host serves is a local start");
    let AgentSelection::LocalStart(permit) = selection else {
        panic!("an endpoint nothing answers on cannot be an external client");
    };
    assert!(
        permit.serves_tcp("127.0.0.1", port),
        "the permit covers the address it was issued for"
    );
    assert!(
        !permit.serves_tcp("127.0.0.1", port + 1),
        "a permit for one port cannot authorize another"
    );
    assert!(
        !permit.serves_tcp("::1", port),
        "a permit for IPv4 loopback cannot authorize IPv6 loopback"
    );
    permit
        .recheck()
        .expect("the home and lock did not move while the permit was held");

    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// Bracket IPv6 loopback is the same permission, parsed the way the Agent's
/// `--grpc-addr` parses it.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn a_serving_host_accepts_bracketed_ipv6_loopback() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    let port = std::net::TcpListener::bind("[::1]:0")
        .map(|listener| listener.local_addr().unwrap().port())
        .unwrap_or(47999);
    let selection = select_agent_serving(
        &format!("[::1]:{port}"),
        "1.2.3",
        Duration::from_millis(500),
        Duration::from_millis(500),
    )
    .await
    .expect("an absent IPv6 loopback endpoint this host serves is a local start");
    let AgentSelection::LocalStart(permit) = selection else {
        panic!("an endpoint nothing answers on cannot be an external client");
    };
    assert!(permit.serves_tcp("::1", port));

    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// The failure class a platform reports for a loopback port nothing listens on
/// is not portable: Unix refuses it immediately, Windows can report the same
/// empty endpoint as a connect timeout (CI measured
/// `ConnectError { kind: Some(TimedOut), details: "... transport error" }` on
/// `127.0.0.1:59051` and `[::1]:59040` with nothing listening). This builds that
/// class with the production connect classifier on any platform — a connector
/// that never completes, bounded by the connect budget — so the decision below
/// is driven by the failure a Windows runner hands back instead of a hand-made
/// error value.
async fn connect_that_never_answered(address: &str) -> ConnectionError {
    let error = transport::connect_channel_with_connector(
        Some(address),
        Duration::from_millis(200),
        |_| std::future::pending(),
    )
    .await
    .err()
    .expect("a connector that never answers cannot produce a channel");
    assert_eq!(
        error.kind,
        Some(std::io::ErrorKind::TimedOut),
        "the premise: an unanswered connect is a timeout: {error}"
    );
    ConnectionError::Transport(error)
}

/// An endpoint this host serves that never answered is this host's to fill, on
/// every platform. Treating only an immediate refusal as "nothing is there" made
/// the serving mode unusable on a Windows runner, where the same empty loopback
/// port is reported as a timeout.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn a_timed_out_loopback_endpoint_this_host_serves_is_a_local_start() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let address = format!("127.0.0.1:{port}");

    let selection = select_from(
        &address,
        Err(connect_that_never_answered(&address).await),
        BindPermission::Serve,
    )
    .expect("an endpoint that never answered is this host's to fill");
    let AgentSelection::LocalStart(permit) = selection else {
        panic!("an endpoint nothing answers on cannot be an external client");
    };
    assert!(
        permit.serves_tcp("127.0.0.1", port),
        "the permit covers the address it was issued for"
    );

    // The exception is the serving policy, not a new reading of timeouts: an
    // attach-only caller is still refused, in the words it was refused in
    // before.
    let error = select_from(
        &address,
        Err(connect_that_never_answered(&address).await),
        BindPermission::AttachOnly,
    )
    .err()
    .expect("an attach-only caller may not start an Agent on an explicit endpoint");
    assert!(
        matches!(error, ConnectionError::StartupBlocked(ref reason) if reason.contains("no local fallback")),
        "{error}"
    );

    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// A timeout says nothing about the local IPC endpoint: that socket either
/// exists or it does not, so the platform's refusal-versus-timeout difference
/// must not reach it.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn a_timed_out_local_endpoint_is_not_a_start_permit() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    let error = select_from(
        "auto",
        Err(connect_that_never_answered("http://127.0.0.1:12345").await),
        BindPermission::AttachOnly,
    )
    .err()
    .expect("a timeout on the IPC endpoint must not start an Agent");
    assert!(
        matches!(error, ConnectionError::Transport(_)),
        "the transport class is preserved, not turned into a permit: {error}"
    );

    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// Accepting the connection is not answering it: a peer that never completes
/// readiness fails the handshake (`Unverified`), which no serving policy turns
/// into a start permit. This is what keeps the timeout case above from becoming
/// "start a second Agent on top of a busy one".
#[tokio::test]
async fn a_listener_that_never_answers_readiness_is_not_a_start_permit() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (stop, mut stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        // A raw listener: it accepts, holds the socket and never speaks HTTP/2.
        let mut held = Vec::new();
        loop {
            tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => match accepted {
                    Ok((socket, _)) => held.push(socket),
                    Err(_) => break,
                },
            }
        }
    });

    let error = select_agent_serving(
        &address,
        "1.2.3",
        Duration::from_millis(300),
        Duration::from_millis(300),
    )
    .await
    .err()
    .expect("a peer that never answers readiness cannot be attached to");
    assert!(
        matches!(error, ConnectionError::Unverified(_)),
        "an unanswered handshake is its own class, not an absent endpoint: {error}"
    );

    let _ = stop.send(());
    task.await.unwrap();
}

/// The same address with attach-only semantics keeps refusing: this is the
/// invariant the serving mode is an explicit exception to, not a rewrite of.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn attaching_only_still_never_fills_an_explicit_endpoint() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let error = select_agent(
        &format!("127.0.0.1:{port}"),
        "1.2.3",
        Duration::from_millis(500),
        Duration::from_millis(500),
    )
    .await
    .err()
    .expect("an attach-only caller may not start an Agent on an explicit endpoint");
    assert!(
        matches!(error, ConnectionError::StartupBlocked(ref reason) if reason.contains("no local fallback")),
        "{error}"
    );

    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// Serving a remote address is refused by name: this process cannot bind it, and
/// starting a local Agent for it would be the redirection the attach-only path
/// exists to prevent.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn serving_a_non_loopback_address_is_refused() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());

    let error = select_agent_serving(
        "10.255.255.1:47200",
        "1.2.3",
        Duration::from_millis(500),
        Duration::from_millis(500),
    )
    .await
    .err()
    .expect("a remote address is not this host's to serve");
    assert!(
        matches!(error, ConnectionError::StartupBlocked(ref reason) if reason.contains("only a loopback address")),
        "{error}"
    );

    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// Serving means naming an address. Automatic discovery is the local IPC
/// endpoint, which the serving mode is precisely the alternative to.
///
/// The endpoint is redirected at an absent socket on purpose: against the real
/// per-user endpoint this test would attach to whatever Agent is serving on the
/// machine running it, and then assert about the wrong branch.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn serving_without_an_explicit_address_is_refused() {
    let _guard = crate::home::env_guard();
    let home = tempfile::tempdir().unwrap();
    let previous_home = std::env::var_os(crate::home::FUTURE_HOME_ENV);
    let previous_socket = std::env::var_os("FUTURE_AGENT_SOCKET");
    std::env::set_var(crate::home::FUTURE_HOME_ENV, home.path());
    std::env::set_var("FUTURE_AGENT_SOCKET", home.path().join("absent.sock"));

    let error = select_agent_serving(
        "auto",
        "1.2.3",
        Duration::from_millis(500),
        Duration::from_millis(500),
    )
    .await
    .err()
    .expect("serving needs an address, not automatic discovery");
    assert!(
        matches!(error, ConnectionError::StartupBlocked(ref reason) if reason.contains("explicit TCP address")),
        "{error}"
    );

    match previous_socket {
        Some(value) => std::env::set_var("FUTURE_AGENT_SOCKET", value),
        None => std::env::remove_var("FUTURE_AGENT_SOCKET"),
    }
    match previous_home {
        Some(value) => std::env::set_var(crate::home::FUTURE_HOME_ENV, value),
        None => std::env::remove_var(crate::home::FUTURE_HOME_ENV),
    }
}

/// The bind address shares one parser with the Agent, so every form the Agent
/// accepts is understood here and nothing is resolved behind the operator's back.
#[test]
fn bind_addresses_parse_the_way_the_agent_parses_them() {
    use std::net::SocketAddr;
    assert_eq!(
        transport::parse_bind_addr("47200").unwrap(),
        "127.0.0.1:47200".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(
        transport::parse_bind_addr(":47200").unwrap(),
        "127.0.0.1:47200".parse::<SocketAddr>().unwrap()
    );
    assert_eq!(
        transport::parse_bind_addr("[::1]:47200").unwrap(),
        "[::1]:47200".parse::<SocketAddr>().unwrap()
    );
    for refused in ["localhost:47200", "::1:47200", "127.0.0.1:70000", "", "abc"] {
        assert!(
            transport::parse_bind_addr(refused).is_err(),
            "{refused:?} must not be guessed at"
        );
    }
    // One grammar, two surfaces: the shared message names what is wrong with
    // the address, and each caller adds the flag it took the value from.
    let refusal = transport::parse_bind_addr("banana").unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "invalid TCP address \"banana\": expected IP:port (bracket IPv6 addresses)"
    );
    assert_eq!(
        refusal.for_flag("--grpc-addr"),
        "invalid --grpc-addr \"banana\": expected IP:port (bracket IPv6 addresses)"
    );
    assert_eq!(refusal.addr(), "banana");
    assert_eq!(
        transport::parse_bind_addr("::1:1234")
            .unwrap_err()
            .to_string(),
        "invalid TCP address \"::1:1234\": bracket IPv6 addresses as [::1]:port"
    );
}
