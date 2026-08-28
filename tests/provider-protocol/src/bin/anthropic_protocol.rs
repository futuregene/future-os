use anyhow::{ensure, Context, Result};
use future_agent::llm::schema::{ModelRequest, ModelStreamEvent};
use future_agent::types::{AgentMessage, ContentBlock, FunctionDef, ToolDef};
use future_provider_protocol_tests::{
    assert_no_stream_error, read_fixture, request_json, start_server, stream_anthropic_events,
    user_request, MockResponse,
};

const MODEL: &str = "claude-sonnet-5";

#[tokio::main]
async fn main() -> Result<()> {
    adaptive_thinking_and_roundtrip_order().await?;
    cache_usage_replays().await?;
    println!("PASS anthropic-protocol: thinking, tool-result order, and usage replays completed");
    Ok(())
}

async fn adaptive_thinking_and_roundtrip_order() -> Result<()> {
    let first_sse = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":4,\"cache_read_input_tokens\":3,\"output_tokens\":0}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"redacted_thinking\",\"data\":\"opaque-redacted\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\",\"signature\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"visible thought\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"signed-thinking\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"lookup\",\"input\":{}}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"q\\\":\\\"x\\\"}\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":2}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":8,\"output_tokens_details\":{\"thinking_tokens\":5}}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let second_sse = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let (base_url, server) = start_server(
        vec![MockResponse::sse(first_sse), MockResponse::sse(second_sse)],
        "/v1",
    )
    .await?;

    let mut first_request = user_request(MODEL, "use the lookup tool");
    first_request.tools.push(tool_def());
    let first_events = stream_anthropic_events(base_url.clone(), MODEL, first_request).await?;
    assert_no_stream_error(&first_events)?;
    let usage = first_events
        .iter()
        .find_map(|event| match event {
            ModelStreamEvent::Finish {
                usage: Some(usage), ..
            } => Some(usage),
            _ => None,
        })
        .context("missing Anthropic finish usage")?;
    ensure!(usage.prompt_tokens == 17, "cache tokens were not included");
    ensure!(
        usage.reasoning_tokens == Some(5),
        "thinking tokens were not captured"
    );

    let reasoning = first_events
        .iter()
        .filter_map(|event| match event {
            ModelStreamEvent::ReasoningEnd {
                id,
                provider_metadata,
            } => {
                let text = if id == "thinking-1" {
                    "visible thought"
                } else {
                    ""
                };
                Some(ContentBlock::reasoning(text, provider_metadata.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    ensure!(reasoning.len() == 2, "thinking blocks were not preserved");

    let mut assistant_content = reasoning;
    assistant_content.push(ContentBlock::tool_call(
        "toolu_1",
        "lookup",
        serde_json::json!({"q": "x"}),
        Default::default(),
    ));
    let second_request = ModelRequest {
        model: MODEL.into(),
        system_prompt: String::new(),
        messages: vec![
            AgentMessage::new_user("user", serde_json::json!("use the lookup tool")),
            AgentMessage {
                role: "assistant".into(),
                content: assistant_content,
                ..Default::default()
            },
            AgentMessage {
                role: "user".into(),
                content: vec![
                    ContentBlock::text("extra context"),
                    ContentBlock::tool_result("toolu_1", "result", false),
                ],
                ..Default::default()
            },
        ],
        tools: vec![tool_def()],
    };
    let second_events = stream_anthropic_events(base_url, MODEL, second_request).await?;
    assert_no_stream_error(&second_events)?;

    let requests = server.await.context("mock server task failed")??;
    let first_body = request_json(&requests[0])?;
    ensure!(first_body["thinking"]["type"] == "adaptive");
    ensure!(first_body["thinking"]["display"] == "summarized");
    let second_body = request_json(&requests[1])?;
    let messages = second_body["messages"]
        .as_array()
        .context("messages is not an array")?;
    let assistant = messages
        .iter()
        .find(|message| message["role"] == "assistant")
        .context("assistant replay message missing")?;
    let assistant_types = assistant["content"]
        .as_array()
        .context("assistant content is not an array")?
        .iter()
        .map(|block| block["type"].as_str().unwrap_or(""))
        .collect::<Vec<_>>();
    ensure!(
        assistant_types == ["redacted_thinking", "thinking", "tool_use"],
        "thinking/tool order changed: {assistant_types:?}"
    );
    let user = messages.last().context("follow-up user message missing")?;
    ensure!(user["content"][0]["type"] == "tool_result");
    ensure!(user["content"][1]["type"] == "text");
    Ok(())
}

async fn cache_usage_replays() -> Result<()> {
    for (fixture, expected_read, expected_write) in [
        ("usage_cache_stream/response_000.txt", Some(4202), Some(0)),
        (
            "usage_cache_creation_stream/response_000.txt",
            Some(0),
            Some(4202),
        ),
    ] {
        let body = read_fixture(&format!("rust-genai/tests/data/yakbak/anthropic/{fixture}"))?;
        let (base_url, server) = start_server(vec![MockResponse::sse(body)], "/v1").await?;
        let events = stream_anthropic_events(
            base_url,
            "claude-haiku-4-5-20251001",
            user_request("claude-haiku-4-5-20251001", "usage fixture"),
        )
        .await?;
        server.await.context("mock server task failed")??;
        assert_no_stream_error(&events)?;
        let usage = events
            .iter()
            .find_map(|event| match event {
                ModelStreamEvent::Finish {
                    usage: Some(usage), ..
                } => Some(usage),
                _ => None,
            })
            .context("missing usage in rust-genai Anthropic replay")?;
        ensure!(usage.prompt_tokens == 4211);
        ensure!(usage.cache_read_tokens == expected_read);
        ensure!(usage.cache_write_tokens == expected_write);
    }
    Ok(())
}

fn tool_def() -> ToolDef {
    ToolDef {
        tool_type: "function".into(),
        function: FunctionDef {
            name: "lookup".into(),
            description: "look something up".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {"q": {"type": "string"}},
                "required": ["q"]
            }),
        },
    }
}
