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
    crate::test_support::ensure_fd_headroom();
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
                        // The response body is complete, so finish our half of the
                        // connection: a cleanly completed response would otherwise
                        // be pooled by the client (and this close never observed),
                        // while a client that gave up releases the connection. The
                        // read below is the assertion that it did.
                        let _ = socket.shutdown().await;
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(_) => panic!("unexpected request data after the response body"),
                        }
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

/// `count` complete SSE text-delta frames, or the same without the final
/// frame's terminating blank line (a response that ends mid-frame).
fn delta_frames(count: usize, terminated: bool) -> &'static str {
    let mut body = String::new();
    for index in 0..count {
        body.push_str(&format!(
            "data: {{\"type\":\"response.output_text.delta\",\"output_index\":1,\"item_id\":\"msg_1\",\"delta\":\"{index}\"}}\n\n"
        ));
    }
    assert!(body.ends_with("\n\n"));
    if !terminated {
        body.truncate(body.len() - 2);
    }
    String::leak(body)
}

/// A consumer that stops reading while the provider is still streaming must let
/// the pump stop and release the HTTP connection. The pump's channel holds 32
/// events, so a 40-frame response parks it inside its `send` loop; dropping the
/// stream there is what closes the connection — the server task below asserts it
/// observed the hang-up, and only a pump that gave up can produce it.
#[tokio::test(flavor = "current_thread")]
async fn an_abandoned_consumer_stops_the_pump_mid_response() {
    let (base, tx, server) = controlled_server().await;
    let client = Client::from_target(protocol_target(&base, responses()));
    let mut stream = client.stream_model(canonical_request()).await.unwrap();
    tx.send(delta_frames(40, true)).await.unwrap();
    // Reading one event proves the pump is running and has filled the channel,
    // so it is parked in a send with frames still unsent.
    assert!(stream.next().await.is_some());
    drop(stream);
    let_the_pump_notice_the_closed_consumer().await;
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("an abandoned response must release the connection")
        .unwrap();
    drop(tx);
}

/// Park the pump in `send` and then leave it idle with the channel full.
///
/// The first chunk carries one event more than the channel holds, so the pump
/// blocks on the last one; reading a single event frees exactly one slot, and the
/// yield that follows lets the woken send complete. The pump then has nothing
/// left to send and nothing left to read, i.e. it is waiting on the wire with
/// every slot occupied — the state in which the EOF phase's sends must block.
async fn fill_the_channel_and_idle_the_pump(
    stream: &mut (impl futures::Stream<Item = ModelStreamEvent> + Unpin),
    tx: &mpsc::Sender<&'static str>,
    body: &'static str,
) {
    tx.send(body).await.unwrap();
    assert!(
        stream.next().await.is_some(),
        "the pump must deliver at least one event"
    );
    tokio::task::yield_now().await;
}

/// Let the socket deliver the end of the response and the pump reach its EOF
/// phase. Real time is required: the runtime polls the I/O driver while a timer
/// is pending, which is what delivers the FIN that a pure task yield misses.
async fn let_the_wire_finish() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// Give the runtime the chance to run the pump task that the receiver drop just
/// woke. Without a yield the root future can resolve on its first poll (the
/// server half-closes, so the client's connection task finishes on its own) and
/// the woken pump task would be dropped at shutdown instead of observing the
/// closed channel.
async fn let_the_pump_notice_the_closed_consumer() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// The same abandonment while the response is *ending*: with a full channel the
/// pump cannot flush the adapter's synthetic closing error, so the EOF path must
/// give up instead of buffering the tail forever.
#[tokio::test(flavor = "current_thread")]
async fn an_abandoned_consumer_stops_the_pump_at_eof() {
    let (base, tx, server) = controlled_server().await;
    let client = Client::from_target(protocol_target(&base, responses()));
    let mut stream = client.stream_model(canonical_request()).await.unwrap();
    // 32 complete frames decode to 33 events: one more than the channel holds.
    fill_the_channel_and_idle_the_pump(&mut stream, &tx, delta_frames(32, true)).await;
    drop(tx);
    let_the_wire_finish().await;
    drop(stream);
    let_the_pump_notice_the_closed_consumer().await;
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("an abandoned response must release the connection")
        .unwrap();
    let _ = base;
}

/// A response that ends *mid-frame* leaves the decoder one residual frame, which
/// the pump decodes and sends after EOF. With the channel full that send is the
/// one that must notice the abandoned consumer.
#[tokio::test(flavor = "current_thread")]
async fn an_abandoned_consumer_stops_the_pump_on_a_residual_frame() {
    let (base, tx, server) = controlled_server().await;
    let client = Client::from_target(protocol_target(&base, responses()));
    let mut stream = client.stream_model(canonical_request()).await.unwrap();
    fill_the_channel_and_idle_the_pump(&mut stream, &tx, delta_frames(33, false)).await;
    drop(tx);
    let_the_wire_finish().await;
    drop(stream);
    let_the_pump_notice_the_closed_consumer().await;
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("an abandoned response must release the connection")
        .unwrap();
    let _ = base;
}

/// Which adapters can carry a usage figure into the EOF finish decides whether
/// the pump's `Usage`-before-`Error` conversion is reachable at all. Only the
/// Anthropic adapter accumulates per-block counts and reports them even for an
/// unterminated stream (all zero, since no frame declared any); the two
/// OpenAI-shaped protocols pass `None`, because with no terminal frame there is
/// nothing to report.
#[test]
fn only_anthropic_carries_a_usage_placeholder_into_the_eof_finish() {
    let registry = crate::llm::adapters::AdapterRegistry::default();
    let mut carried = Vec::new();
    for protocol in [
        schema::ApiProtocol::OpenAiResponses,
        schema::ApiProtocol::OpenAiChatCompletions,
        schema::ApiProtocol::AnthropicMessages,
    ] {
        let adapter = registry.get(protocol).unwrap();
        let mut state = adapter.new_stream_state();
        let events = adapter.finish_stream(state.as_mut()).unwrap();
        let finish = events.iter().find(|event| {
            matches!(
                event,
                ModelStreamEvent::Finish {
                    reason: FinishReason::Incomplete,
                    ..
                }
            )
        });
        let Some(ModelStreamEvent::Finish { usage, .. }) = finish else {
            panic!(
                "{} must close an unterminated stream with an incomplete finish: {events:?}",
                protocol.canonical_name()
            );
        };
        if let Some(usage) = usage {
            assert_eq!(
                usage.total_tokens,
                0,
                "{} invented a token count for a stream that declared none",
                protocol.canonical_name()
            );
            carried.push(protocol.canonical_name());
        }
    }
    assert_eq!(
        carried,
        vec![schema::ApiProtocol::AnthropicMessages.canonical_name()],
        "the pump's usage-then-error conversion is reachable only through the adapter that carries usage"
    );
}

/// An Anthropic response with one text block opened, `count` deltas after it, and
/// (when `close`) the block stopped again. Closing it is what makes the pump's
/// finish phase begin with the usage send rather than with a block-closing event.
fn anthropic_body(count: usize, close: bool) -> &'static str {
    let mut body = String::from(
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n\
         data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    );
    for index in 0..count {
        body.push_str(&format!(
            "data: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{index}\"}}}}\n\n"
        ));
    }
    if close {
        body.push_str("data: {\"type\":\"content_block_stop\",\"index\":0}\n\n");
    }
    String::leak(body)
}

/// How many events the body above decodes to, read from the registered adapter so
/// the pump fixtures below cannot drift from the decoder. Also returns the events
/// `finish_stream` still owes afterwards.
fn anthropic_decode(count: usize, close: bool) -> (usize, Vec<ModelStreamEvent>) {
    let registry = crate::llm::adapters::AdapterRegistry::default();
    let adapter = registry
        .get(schema::ApiProtocol::AnthropicMessages)
        .unwrap();
    let mut decoder = crate::llm::sse::SseDecoder::default();
    let mut state = adapter.new_stream_state();
    let frames = decoder
        .push(anthropic_body(count, close).as_bytes())
        .unwrap();
    let mut events = 0;
    for frame in &frames {
        events += adapter.decode_frame(frame, state.as_mut()).unwrap().len();
    }
    let owed = adapter.finish_stream(state.as_mut()).unwrap();
    (events, owed)
}

/// A body whose events fill the channel and leave the pump idle with every slot
/// occupied: events + 1 before EOF, so the confirmatory read above both proves
/// the pump ran and leaves exactly a full channel.
fn anthropic_deltas_for_a_full_channel(close: bool) -> usize {
    let deltas = (1..=64)
        .find(|deltas| anthropic_decode(*deltas, close).0 == 33)
        .expect("a response with one channel plus one event must be expressible");
    assert_eq!(anthropic_decode(deltas, close).0, 33);
    deltas
}

/// The Anthropic EOF finish carries a (zeroed) usage, so a consumer that
/// abandons the stream *at* EOF must be noticed by the usage send that precedes
/// the synthetic error — the second place a full channel has to give up.
#[tokio::test(flavor = "current_thread")]
async fn an_abandoned_consumer_stops_the_pump_before_the_eof_usage_send() {
    let deltas = anthropic_deltas_for_a_full_channel(true);
    // With the block already closed, the finish phase's first send is the usage
    // one; otherwise a block-closing event would be sent first and this fixture
    // would cover the other send instead.
    let (_, owed) = anthropic_decode(deltas, true);
    assert!(
        matches!(
            owed.first(),
            Some(ModelStreamEvent::Finish { usage: Some(_), .. })
        ),
        "the fixture must reach the usage send first: {owed:?}"
    );
    let (base, tx, server) = controlled_server().await;
    let client = Client::from_target(protocol_target(
        &base,
        ProtocolConfig::AnthropicMessages(schema::AnthropicMessagesConfig::default()),
    ));
    let mut stream = client.stream_model(canonical_request()).await.unwrap();
    fill_the_channel_and_idle_the_pump(&mut stream, &tx, anthropic_body(deltas, true)).await;
    drop(tx);
    let_the_wire_finish().await;
    drop(stream);
    let_the_pump_notice_the_closed_consumer().await;
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("an abandoned response must release the connection")
        .unwrap();
    let _ = base;
}

/// The same abandonment with the text block still open: the finish phase's first
/// send is the block-closing event, and that is the send that must notice the
/// abandoned consumer.
#[tokio::test(flavor = "current_thread")]
async fn an_abandoned_consumer_stops_the_pump_on_the_closing_block() {
    let deltas = anthropic_deltas_for_a_full_channel(false);
    let (_, owed) = anthropic_decode(deltas, false);
    assert!(
        !matches!(
            owed.first(),
            Some(ModelStreamEvent::Finish { usage: Some(_), .. })
        ),
        "the fixture must send a closing event before the usage one: {owed:?}"
    );
    let (base, tx, server) = controlled_server().await;
    let client = Client::from_target(protocol_target(
        &base,
        ProtocolConfig::AnthropicMessages(schema::AnthropicMessagesConfig::default()),
    ));
    let mut stream = client.stream_model(canonical_request()).await.unwrap();
    fill_the_channel_and_idle_the_pump(&mut stream, &tx, anthropic_body(deltas, false)).await;
    drop(tx);
    let_the_wire_finish().await;
    drop(stream);
    let_the_pump_notice_the_closed_consumer().await;
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("an abandoned response must release the connection")
        .unwrap();
    let _ = base;
}

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
    assert!(matches!(events.as_slice(), [
            ModelStreamEvent::ReasoningEnd { provider_metadata, .. },
            ModelStreamEvent::Error { message },
        ] if provider_metadata["openai"].get("id").is_none()
            && message.contains("kind=timeout") && !message.contains("idle_timeout")));
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
