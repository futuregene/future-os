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
