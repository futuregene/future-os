//! Real HTTP boundary tests for long reasoning pauses, cancellation and SSE completion.
use super::schema::{FinishReason, ModelStreamEvent, ProtocolConfig};
use super::tests::{canonical_request, protocol_target};
use super::*;
use crate::types::LLMProvider;
use futures::FutureExt;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// The response stays open until the sender is dropped. Tests can send new SSE
/// data independently of EOF, and advance Tokio time only after HTTP setup.
async fn controlled_server() -> (
    String,
    mpsc::Sender<&'static str>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, mut rx) = mpsc::channel::<&'static str>(8);
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buf = [0; 4096];
        loop {
            let n = socket.read(&mut buf).await.unwrap();
            assert_ne!(n, 0, "request ended before headers/body");
            request.extend_from_slice(&buf[..n]);
            if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        loop {
            tokio::select! {
                data = rx.recv() => {
                    let Some(data) = data else {
                        let _ = socket.write_all(b"0\r\n\r\n").await;
                        return;
                    };
                    let wire = format!("{:X}\r\n{data}\r\n", data.len());
                    if socket.write_all(wire.as_bytes()).await.is_err() {
                        return;
                    }
                }
                // The client should release the connection on cancellation or
                // protocol completion, even when the server sends no more data.
                read = socket.read(&mut buf) => {
                    assert!(matches!(read, Ok(0) | Err(_)), "unexpected second request");
                    return;
                }
            }
        }
    });
    (base, tx, task)
}

fn responses() -> ProtocolConfig {
    ProtocolConfig::OpenAiResponses(schema::OpenAiResponsesConfig::default())
}

const REASONING_START: &str = "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"id\":\"rs_1\",\"summary\":[]}}\n\n";
const RESPONSE_DONE: &str = concat!(
    "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"id\":\"rs_1\",\"summary\":[],\"encrypted_content\":\"cipher\"}}\n\n",
    "data: {\"type\":\"response.output_text.delta\",\"output_index\":1,\"item_id\":\"msg_1\",\"delta\":\"answer\"}\n\n",
    "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":2,\"output_tokens\":3,\"total_tokens\":5}}}\n\n",
);

#[tokio::test(flavor = "current_thread")]
async fn long_silence_in_http_body_preserves_reasoning_and_completion() {
    let (base, tx, server) = controlled_server().await;
    let client = Client::from_target(protocol_target(&base, responses()));
    tx.send(REASONING_START).await.unwrap();
    let mut stream = client.stream_model(canonical_request()).await.unwrap();
    assert!(matches!(
        stream.next().await,
        Some(ModelStreamEvent::ReasoningStart { .. })
    ));

    // Advance only after the first event has crossed the real HTTP boundary:
    // no sleeps or test-only timeout overrides in production code are needed.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(300)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    assert!(
        stream.next().now_or_never().is_none(),
        "silence is not a stream error"
    );

    tx.send(RESPONSE_DONE).await.unwrap();
    drop(tx);
    let events: Vec<_> = stream.collect().await;
    assert!(events.iter().any(|event| matches!(event,
        ModelStreamEvent::ReasoningEnd { provider_metadata, .. }
        if provider_metadata["openai"]["encrypted_content"] == "cipher")));
    assert!(events.iter().any(|event| matches!(event,
        ModelStreamEvent::TextDelta { text, .. } if text == "answer")));
    assert!(
        matches!(events.last(), Some(ModelStreamEvent::Finish { reason: FinishReason::Stop, usage: Some(usage) }) if usage.total_tokens == 5)
    );
    server.await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn protocol_terminal_closes_http_without_waiting_for_eof() {
    let cases = [
        (responses(), RESPONSE_DONE),
        (responses(), "data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n\n"),
        (responses(), "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"provider failed\"}}}\n\n"),
        (ProtocolConfig::AnthropicMessages(schema::AnthropicMessagesConfig::default()), "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\ndata: {\"type\":\"message_stop\"}\n\n"),
        (schema::ResolvedModelTarget::openai_chat_compatible("mock", "http://localhost", "secret", None, None).protocol,
         "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"),
        (schema::ResolvedModelTarget::openai_chat_compatible("mock", "http://localhost", "secret", None, None).protocol,
         "data: {\"error\":{\"message\":\"provider failed\"}}\n\n"),
    ];
    for (protocol, data) in cases {
        let (base, tx, server) = controlled_server().await;
        let client = Client::from_target(protocol_target(&base, protocol));
        tx.send(data).await.unwrap();
        let stream = client.stream_model(canonical_request()).await.unwrap();
        let events: Vec<_> = tokio::time::timeout(Duration::from_secs(2), stream.collect())
            .await
            .expect("protocol terminal should close the receiver without HTTP EOF");
        assert!(events.iter().any(|e| matches!(
            e,
            ModelStreamEvent::Finish { .. } | ModelStreamEvent::Error { .. }
        )));
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        drop(tx);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn chat_finish_waits_for_delayed_usage_and_done() {
    let (base, tx, server) = controlled_server().await;
    let client = Client::from_target(schema::ResolvedModelTarget::openai_chat_compatible(
        "mock", &base, "secret", None, None,
    ));
    tx.send("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n")
        .await
        .unwrap();
    let mut stream = client.stream_model(canonical_request()).await.unwrap();
    assert!(matches!(
        stream.next().await,
        Some(ModelStreamEvent::Finish { .. })
    ));
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(300)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    assert!(
        stream.next().now_or_never().is_none(),
        "finish_reason is not the SSE terminator"
    );
    tx.send(concat!(
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":62,\"completion_tokens\":29,\"total_tokens\":91}}\n\n",
        "data: [DONE]\n\n",
    )).await.unwrap();
    let events: Vec<_> = tokio::time::timeout(Duration::from_secs(2), stream.collect())
        .await
        .unwrap();
    assert!(
        matches!(events.as_slice(), [ModelStreamEvent::Usage(usage)] if usage.total_tokens == 91)
    );
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    drop(tx);
}

#[tokio::test(flavor = "current_thread")]
async fn total_request_deadline_still_ends_a_silent_http_stream() {
    let (base, tx, server) = controlled_server().await;
    let client = Client::from_target(protocol_target(&base, responses()));
    tx.send(REASONING_START).await.unwrap();
    let mut stream = client.stream_model(canonical_request()).await.unwrap();
    assert!(matches!(
        stream.next().await,
        Some(ModelStreamEvent::ReasoningStart { .. })
    ));
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(llm_timeout_secs() + 1)).await;
    tokio::task::yield_now().await;
    tokio::time::resume();
    let events: Vec<_> = tokio::time::timeout(Duration::from_secs(2), stream.collect())
        .await
        .unwrap();
    assert!(
        matches!(events.as_slice(), [ModelStreamEvent::Error { message }]
        if message.contains("kind=timeout") && !message.contains("idle_timeout"))
    );
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    drop(tx);
}

#[tokio::test(flavor = "current_thread")]
async fn consumer_cancellation_closes_a_completely_silent_http_stream() {
    let (base, tx, server) = controlled_server().await;
    let client = Client::from_target(protocol_target(&base, responses()));
    let stream = client.stream_model(canonical_request()).await.unwrap();
    drop(stream);
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    drop(tx);
}
