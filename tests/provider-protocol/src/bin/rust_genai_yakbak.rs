use anyhow::{ensure, Context, Result};
use future_agent::llm::schema::{FinishReason, ModelStreamEvent};
use future_provider_protocol_tests::{
    assert_no_stream_error, read_fixture, start_server, stream_events, user_request, MockResponse,
};

#[tokio::main]
async fn main() -> Result<()> {
    completed_output_empty().await?;
    terminal_output_missing().await?;
    utf8_http_chunk_boundary().await?;
    println!("PASS rust-genai-yakbak: 3 OpenAI Responses replays completed");
    Ok(())
}

async fn completed_output_empty() -> Result<()> {
    let body = read_fixture(
        "rust-genai/tests/data/yakbak/openai_resp/reasoning_stream_completed_empty/response_000.txt",
    )?;
    let events = replay(body, Vec::new()).await?;
    assert_no_stream_error(&events)?;
    ensure!(events
        .iter()
        .any(|event| matches!(event, ModelStreamEvent::ReasoningEnd { .. })));
    ensure!(events.iter().any(|event| matches!(
        event,
        ModelStreamEvent::Finish {
            reason: FinishReason::Stop,
            ..
        }
    )));
    Ok(())
}

async fn terminal_output_missing() -> Result<()> {
    let body = read_fixture(
        "rust-genai/tests/data/yakbak/openai_resp/tools_completed_no_output_field/response_000.txt",
    )?;
    let events = replay(body, Vec::new()).await?;
    assert_no_stream_error(&events)?;
    ensure!(events
        .iter()
        .any(|event| matches!(event, ModelStreamEvent::ToolInputEnd { .. })));
    ensure!(events
        .iter()
        .any(|event| matches!(event, ModelStreamEvent::Finish { .. })));
    Ok(())
}

async fn utf8_http_chunk_boundary() -> Result<()> {
    let body = read_fixture(
        "rust-genai/tests/data/yakbak/openai_resp/utf8_chunking_bug/response_000.txt",
    )?;
    let needle = "日本語".as_bytes();
    let start = body
        .windows(needle.len())
        .position(|window| window == needle)
        .context("UTF-8 fixture no longer contains 日本語")?;
    // Split after the first byte of 日 to force reqwest/SSE buffering across
    // an invalid standalone UTF-8 prefix.
    let events = replay(body, vec![start + 1]).await?;
    assert_no_stream_error(&events)?;
    let text = events
        .iter()
        .filter_map(|event| match event {
            ModelStreamEvent::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    ensure!(
        text.contains("日本語のテスト"),
        "UTF-8 content was corrupted: {text}"
    );
    Ok(())
}

async fn replay(body: Vec<u8>, chunk_ends: Vec<usize>) -> Result<Vec<ModelStreamEvent>> {
    let response = if chunk_ends.is_empty() {
        MockResponse::sse(body)
    } else {
        MockResponse::chunked_sse(body, chunk_ends)
    };
    let (base_url, server) = start_server(vec![response], "/v1").await?;
    let events = stream_events(
        base_url,
        "gpt-5.4-mini",
        "low",
        user_request("gpt-5.4-mini", "fixture replay"),
    )
    .await?;
    server.await.context("mock server task failed")??;
    Ok(events)
}
