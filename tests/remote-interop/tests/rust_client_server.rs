use future_remote_client::{host::ClientHost, Client};
use future_remote_interop::{contract, Broker, Fixture, MemoryClientHost, DESKTOP};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn public_rust_roles_pair_request_publish_transfer_and_reconnect() -> anyhow::Result<()> {
    let suite = contract::suite()?;
    let broker = Broker::start().await?;
    let fixture = Fixture::start(&broker.nats_url, &broker.websocket_url).await?;
    let host = Arc::new(MemoryClientHost::new(fixture.client_credentials.clone()));
    let client = Client::new(host.clone());
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let emitter: future_remote_client::Emitter = Arc::new(move |event| {
        let _ = events.send(event);
    });
    let status = client
        .connect_with_emitter(DESKTOP, Some(emitter.clone()))
        .await?;
    assert!(status.connected);
    assert!(fixture.metrics()["identityBound"].as_bool().unwrap());
    assert!(host.load()?.peers[0]
        .creds
        .secure
        .as_ref()
        .unwrap()
        .secret
        .is_none());
    println!("remote-interop rust {} PASS", suite.flow_ids.first_pair);

    for case in &suite.cases {
        for step in &case.steps {
            let result = client.request(DESKTOP, step.command.clone(), "list").await;
            match (&step.data, &step.error, result) {
                (Some(expected), None, Ok(actual)) => assert_eq!(&actual, expected, "{}", case.id),
                (None, Some(expected), Err(error)) => {
                    assert_eq!(&error.to_string(), expected, "{}", case.id)
                }
                _ => anyhow::bail!("shared case outcome mismatch: {}", case.id),
            }
        }
        let id = case.steps[0].command["id"].as_str().unwrap();
        assert_eq!(
            fixture.metrics()["executions"][id],
            json!(case.expected_executions),
            "{}",
            case.id
        );
        println!("remote-interop rust {} PASS", case.id);
    }
    fixture.emit()?;
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = received.recv().await.expect("event stream closed");
            if event.kind == "event" && event.payload["type"] == suite.event.kind {
                break event;
            }
        }
    })
    .await?;
    assert_eq!(event.payload["runId"], suite.event.run_id);
    assert_eq!(event.payload["idx"], suite.event.idx);
    assert_eq!(event.payload["data"], suite.event.data);
    println!("remote-interop rust {} PASS", suite.flow_ids.event);

    let upload = client
        .upload(DESKTOP, "fixture.bin", &suite.file_bytes)
        .await?;
    assert_eq!(
        upload.content_hash,
        format!("{:x}", Sha256::digest(&suite.file_bytes))
    );
    let download = client
        .download(
            DESKTOP,
            &suite.event.session_id,
            "fixture.bin",
            Some("fixture.bin"),
            "original",
        )
        .await?;
    assert_eq!(download.bytes, suite.file_bytes);
    // Download cancellation is part of the actual Client transfer operation.
    assert_eq!(fixture.metrics()["downloads"], 0);
    println!("remote-interop rust {} PASS", suite.flow_ids.transfer);

    client.disconnect(DESKTOP).await;
    client.connect_with_emitter(DESKTOP, Some(emitter)).await?;
    let result = client
        .request(
            DESKTOP,
            json!({"id":"reconnect", "type":"get_state", "message":"IK"}),
            "list",
        )
        .await?;
    assert_eq!(result, json!({"echo":"IK", "kind":"get_state"}));
    assert!(fixture.metrics()["identityBound"].as_bool().unwrap());
    println!("remote-interop rust {} PASS", suite.flow_ids.reconnect);
    client.shutdown(Duration::from_secs(5)).await?;
    fixture.shutdown().await?;
    assert!(client.is_stopped());
    println!("remote-interop rust {} PASS", suite.flow_ids.shutdown);
    Ok(())
}

#[tokio::test]
async fn one_rust_client_keeps_two_real_servers_separate() -> anyhow::Result<()> {
    let suite = contract::isolation()?;
    let broker = Broker::start().await?;
    let first = Fixture::start_named(
        &broker.nats_url,
        &broker.websocket_url,
        &suite.endpoints[0].label,
    )
    .await?;
    let second = Fixture::start_named(
        &broker.nats_url,
        &broker.websocket_url,
        &suite.endpoints[1].label,
    )
    .await?;
    let fixtures = [&first, &second];
    let host = Arc::new(MemoryClientHost::new(first.client_credentials.clone()));
    host.upsert(second.client_credentials.clone())?;
    let client = Client::new(host.clone());
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let emitter: future_remote_client::Emitter = Arc::new(move |event| {
        let _ = events.send(event);
    });
    let result: anyhow::Result<()> = async {
        let a = &first.client_credentials.desktop_id;
        let b = &second.client_credentials.desktop_id;
        let (a_status, b_status) = tokio::try_join!(
            client.connect_with_emitter(a, Some(emitter.clone())),
            client.connect_with_emitter(b, Some(emitter.clone()))
        )?;
        assert!(a_status.connected && b_status.connected);
        assert_eq!(client.list().await?.len(), 2);
        for (index, fixture) in fixtures.iter().enumerate() {
            let command = json!({"id":suite.request_id, "type":"get_state", "message":suite.endpoints[index].message});
            // The same request ID is independent on each real endpoint. Replaying
            // one endpoint's request must hit its own deduplication cache.
            for _ in 0..2 {
                assert_eq!(client.request(&fixture.client_credentials.desktop_id, command.clone(), "list").await?, json!({"echo":suite.endpoints[index].message,"kind":"get_state"}));
            }
            assert_eq!(fixture.metrics()["executions"][&suite.request_id], 1);
            assert_eq!(fixture.metrics()["identityBound"], true);
        }
        println!("remote-interop rust {} PASS", suite.flow_ids.requests);
        for fixture in fixtures {
            let desktop = &fixture.client_credentials.desktop_id;
            let catalog = client.sessions(desktop).await?;
            assert_eq!(catalog["desktopId"], *desktop);
            assert_eq!(catalog["pairId"], fixture.client_credentials.pair_id);
            assert_eq!(catalog["sessions"][0]["title"], *desktop);
        }
        println!("remote-interop rust {} PASS", suite.flow_ids.catalog);
        for (index, fixture) in fixtures.iter().enumerate() {
            fixture.emit_data(&suite.endpoints[index].message)?;
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let event = received.recv().await.expect("isolation event stream closed");
                    if event.kind == "event" {
                        for (source, endpoint) in suite.endpoints.iter().enumerate() {
                            if event.payload["data"] == endpoint.message {
                                assert_eq!(event.desktop_id, fixtures[source].client_credentials.desktop_id);
                            }
                        }
                        if event.payload["data"] == suite.endpoints[index].message {
                            assert_eq!(event.desktop_id, fixture.client_credentials.desktop_id);
                            break;
                        }
                    }
                }
            }).await?;
        }
        println!("remote-interop rust {} PASS", suite.flow_ids.events);
        client.disconnect(a).await;
        let summaries = client.list().await?;
        assert!(summaries.iter().any(|peer| peer.desktop_id == *a && peer.stopped && !peer.connected));
        assert!(summaries.iter().any(|peer| peer.desktop_id == *b && !peer.stopped && peer.connected));
        assert_eq!(client.request(b, json!({"id":"isolation_survivor","type":"get_state","message":suite.endpoints[1].message}), "list").await?, json!({"echo":suite.endpoints[1].message,"kind":"get_state"}));
        assert!(first.metrics()["executions"].get("isolation_survivor").is_none());
        assert_eq!(second.metrics()["executions"]["isolation_survivor"], 1);
        println!("remote-interop rust {} PASS", suite.flow_ids.disconnect);
        client.connect_with_emitter(a, Some(emitter)).await?;
        assert!(client.list().await?.iter().all(|peer| peer.connected && !peer.stopped));
        assert_eq!(client.request(a, json!({"id":"isolation_restored","type":"get_state","message":suite.endpoints[0].message}), "list").await?, json!({"echo":suite.endpoints[0].message,"kind":"get_state"}));
        assert!(second.metrics()["executions"].get("isolation_restored").is_none());
        assert!(host.load()?.peers.iter().all(|peer| peer.creds.secure.as_ref().is_some_and(|identity| identity.secret.is_none())));
        println!("remote-interop rust {} PASS", suite.flow_ids.reconnect);
        Ok(())
    }.await;
    let (client_cleanup, first_cleanup, second_cleanup) = tokio::join!(
        client.shutdown(Duration::from_secs(5)),
        first.shutdown(),
        second.shutdown()
    );
    result?;
    client_cleanup?;
    first_cleanup?;
    second_cleanup?;
    Ok(())
}

#[path = "support/scripted_requests.rs"]
mod scripted_requests;
