//! On-demand materialization of scoped canonical event fields. Normal token
//! traffic never calls this path; tool bodies stay lazy until explicitly read.
use crate::{content_range::ContentRequest, proto};
use serde_json::Value;

type Client = crate::connection::VerifiedAgentClient;
const MAX_MATERIALIZED_BYTES: usize = 16 * 1024 * 1024;

pub fn required_inline(kind: &str) -> bool {
    matches!(
        kind,
        "user_message"
            | "approval_request"
            | "error"
            | "agent_end"
            | "compaction_committed"
            | "compaction_failed"
    )
}

pub fn needs_hydration(kind: &str, data: &Value) -> bool {
    required_inline(kind)
        && data
            .get("content_refs")
            .and_then(Value::as_object)
            .is_some_and(|refs| !refs.is_empty())
}

/// Validate immutable-page identity and byte progress before allocating the
/// complete field. JSON parsing/joining occurs on the blocking pool, outside
/// runtime/stream sequencing locks. Oversized details use individual pages.
pub async fn read_field(
    client: &mut Client,
    session: &str,
    run: &str,
    mut request: ContentRequest,
) -> Result<Value, String> {
    request.offset = 0;
    request.max_bytes = 64 * 1024;
    let mut parts = Vec::new();
    let mut identity: Option<(String, i64, String)> = None;
    loop {
        let response = client
            .execute_command(crate::command_policy::request_with_timeout(
                proto::RpcCommand {
                    id: format!("content:{}:{}", request.cursor, request.offset),
                    r#type: "get_event_content_range".into(),
                    session_id: session.to_owned(),
                    run_id: run.to_owned(),
                    content_request: Some(request.clone().into()),
                    ..Default::default()
                },
            ))
            .await
            .map_err(|error| error.to_string())?
            .into_inner();
        if !response.success {
            return Err(response.error);
        }
        let page = crate::decode::response_data(&response);
        let text = page["text"].as_str().ok_or("invalid content page text")?;
        let next = page["nextOffset"]
            .as_i64()
            .ok_or("invalid content page cursor")?;
        let total = page["totalBytes"]
            .as_i64()
            .filter(|n| *n >= 0 && *n <= MAX_MATERIALIZED_BYTES as i64)
            .ok_or("content field exceeds materialization budget; read individual pages")?;
        let etag = page["etag"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("invalid content page etag")?;
        let encoding = page["encoding"]
            .as_str()
            .filter(|v| matches!(*v, "text" | "json"))
            .ok_or("invalid content page encoding")?;
        let more = page["hasMore"]
            .as_bool()
            .ok_or("invalid content page continuation")?;
        let valid = page["databaseId"] == request.database_id
            && page["restoreGeneration"] == request.restore_generation
            && next == request.offset + text.len() as i64
            && next <= total
            && more == (next < total)
            && (next > request.offset || next == total)
            && text.len() <= request.max_bytes as usize;
        if !valid {
            return Err("invalid content page identity or byte range".into());
        }
        let stamp = (etag.to_owned(), total, encoding.to_owned());
        if identity.as_ref().is_some_and(|old| old != &stamp) {
            return Err("content page changed while reading".into());
        }
        identity = Some(stamp);
        parts.push(text.to_owned());
        request.offset = next;
        if !more {
            break;
        }
        tokio::task::yield_now().await;
    }
    let encoding = identity.expect("at least one page").2;
    tokio::task::spawn_blocking(move || {
        let text = parts.concat();
        if encoding == "json" {
            serde_json::from_str(&text).map_err(|error| error.to_string())
        } else {
            Ok(Value::String(text))
        }
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Hydrate only fields that the live consumer needs for a semantic decision or
/// a user-visible message. Optional tool facts remain explicit lazy references.
pub async fn hydrate_required(
    client: &mut Client,
    session: &str,
    run: &str,
    kind: &str,
    data: &mut Value,
) -> Result<(), String> {
    if !needs_hydration(kind, data) {
        return Ok(());
    }
    let refs = data["content_refs"]
        .as_object()
        .expect("checked references")
        .clone();
    for (name, value) in refs {
        let request: ContentRequest =
            serde_json::from_value(value).map_err(|error| error.to_string())?;
        let expected = format!("/data/{}", name.replace('~', "~0").replace('/', "~1"));
        if request.field != expected {
            return Err("event field reference mismatch".into());
        }
        data[&name] = read_field(client, session, run, request).await?;
        data["content_refs"]
            .as_object_mut()
            .expect("references object")
            .remove(&name);
    }
    data.as_object_mut()
        .ok_or("invalid event object")?
        .remove("content_refs");
    Ok(())
}

pub fn stream_needs_hydration(event: &proto::StreamEvent) -> bool {
    (required_inline(&event.r#type) && event.data.contains("\"content_refs\""))
        || event
            .snapshot_events
            .iter()
            .any(|event| required_inline(&event.r#type) && event.data.contains("\"content_refs\""))
}

pub async fn hydrate_stream_event(
    client: &mut Client,
    session: &str,
    event: &mut proto::StreamEvent,
) -> Result<(), String> {
    if required_inline(&event.r#type) && event.data.contains("\"content_refs\"") {
        let mut data: Value = serde_json::from_str(&crate::decode::event_data_json(event))
            .map_err(|error| error.to_string())?;
        hydrate_required(client, session, &event.run_id, &event.r#type, &mut data).await?;
        event.data = data.to_string();
    }
    for projected in &mut event.snapshot_events {
        if !required_inline(&projected.r#type) || !projected.data.contains("\"content_refs\"") {
            continue;
        }
        let mut data: Value =
            serde_json::from_str(&crate::decode::projected_event_data_json(projected))
                .map_err(|error| error.to_string())?;
        hydrate_required(client, session, &event.run_id, &projected.r#type, &mut data).await?;
        projected.data = data.to_string();
    }
    Ok(())
}
