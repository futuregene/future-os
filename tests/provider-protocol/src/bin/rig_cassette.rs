use anyhow::{ensure, Context, Result};
use future_agent::llm::schema::{FinishReason, ModelStreamEvent};
use future_provider_protocol_tests::{
    assert_no_stream_error, read_rig_cassette, request_json, start_server, stream_events,
    user_request, MockResponse,
};

#[tokio::main]
async fn main() -> Result<()> {
    let cassette = read_rig_cassette(
        "rig/tests/cassettes/openai/raw_stream_capture_matrix/responses_reasoning_stream_raw_round_trips_typed.yaml",
    )?;
    let (base_url, server) = start_server(
        vec![MockResponse {
            status: cassette.then.status,
            body: cassette.then.body.into_bytes(),
            chunk_ends: Vec::new(),
        }],
        "/v1",
    )
    .await?;
    let prompt = "A train leaves at 09:30 and travels 150 km at 60 km/h. At what time does it arrive? Reply with only the time in HH:MM.";
    let events = stream_events(
        base_url,
        "gpt-5.2",
        "medium",
        user_request("gpt-5.2", prompt),
    )
    .await?;
    assert_no_stream_error(&events)?;
    ensure!(
        events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::ReasoningEnd { .. })),
        "Rig response did not produce a reasoning block"
    );
    ensure!(
        events.iter().any(|event| matches!(
            event,
            ModelStreamEvent::Finish {
                reason: FinishReason::Stop,
                ..
            }
        )),
        "Rig response did not finish normally"
    );

    let requests = server.await.context("mock server task failed")??;
    let request = &requests[0];
    ensure!(
        request.method == cassette.when.method,
        "HTTP method changed"
    );
    ensure!(request.path == cassette.when.path, "endpoint changed");
    let actual = request_json(request)?;
    let mut expected: serde_json::Value = serde_json::from_str(&cassette.when.body)?;
    // Rig omits false while FutureOS deliberately sends `store: false`.
    expected["store"] = serde_json::json!(false);
    ensure!(actual == expected, "outbound request no longer matches the Rig cassette\nexpected: {expected}\nactual:   {actual}");

    println!("PASS rig-cassette: request body matched and real SSE replay completed");
    Ok(())
}
