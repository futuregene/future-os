use anyhow::{ensure, Context, Result};
use future_agent::llm::schema::ModelStreamEvent;
use future_agent::types::{AgentMessage, ContentBlock};
use future_provider_protocol_tests::{
    assert_no_stream_error, request_json, start_server, stream_events, user_request, MockResponse,
};

#[tokio::main]
async fn main() -> Result<()> {
    let identity_sse = concat!(
        "event: response.output_item.added\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"reasoning\"}}\n\n",
        "event: response.output_item.done\n",
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"reasoning\",\"summary\":[],\"encrypted_content\":\"idless-cipher\"}}\n\n",
        "event: response.output_item.added\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"reasoning\"}}\n\n",
        "event: response.output_item.done\n",
        "data: {\"type\":\"response.output_item.done\",\"output_index\":1,\"item\":{\"type\":\"reasoning\",\"id\":\"rs_wire\",\"summary\":[],\"encrypted_content\":\"wire-cipher\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n",
    );
    let terminal_sse = concat!(
        "event: response.output_text.delta\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_2\",\"output_index\":0,\"delta\":\"ok\"}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}\n\n",
    );
    let (base_url, server) = start_server(
        vec![
            MockResponse::sse(identity_sse),
            MockResponse::sse(terminal_sse),
        ],
        "/v1",
    )
    .await?;

    let first_events = stream_events(
        base_url.clone(),
        "gpt-5.2",
        "medium",
        user_request("gpt-5.2", "identity test"),
    )
    .await?;
    assert_no_stream_error(&first_events)?;
    let reasoning = first_events
        .iter()
        .filter_map(|event| match event {
            ModelStreamEvent::ReasoningEnd {
                provider_metadata, ..
            } => Some(ContentBlock::reasoning("", provider_metadata.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(reasoning.len() == 2, "expected two reasoning blocks");

    let mut second = user_request("gpt-5.2", "identity test");
    second.messages.push(AgentMessage {
        role: "assistant".into(),
        content: reasoning,
        ..Default::default()
    });
    second.messages.push(AgentMessage::new_user(
        "user",
        serde_json::json!("continue"),
    ));
    let second_events = stream_events(base_url, "gpt-5.2", "medium", second).await?;
    assert_no_stream_error(&second_events)?;

    let requests = server.await.context("mock server task failed")??;
    let second_body = request_json(&requests[1])?;
    let input = second_body["input"]
        .as_array()
        .context("request input is not an array")?;
    let replayed_reasoning = input
        .iter()
        .filter(|item| item.get("type").and_then(serde_json::Value::as_str) == Some("reasoning"))
        .collect::<Vec<_>>();
    ensure!(
        replayed_reasoning.len() == 1,
        "id-less reasoning was replayed"
    );
    ensure!(
        replayed_reasoning[0]["id"] == "rs_wire",
        "wire identity was not preserved"
    );
    ensure!(
        !second_body.to_string().contains("reasoning-0")
            && !second_body.to_string().contains("reasoning-1"),
        "synthetic assembly identity leaked into the request"
    );

    println!("PASS rig-structure: missing IDs stayed local and rs_wire was replayed");
    Ok(())
}
