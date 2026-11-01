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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::AgentSelection;
    use serde_json::json;
    use std::time::Duration;

    const DATABASE: &str = "db-1";
    const GENERATION: &str = "gen-1";

    /// A host that answers `get_event_content_range` with these pages, in order.
    async fn serving(
        pages: Vec<serde_json::Value>,
    ) -> (
        Client,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let mock = crate::connection::tests::mock_agent_answering(pages);
        let (stop, task) = crate::connection::tests::serve_mock(listener, mock).await;
        let selection = crate::connection::select_agent(
            &address,
            "1.2.3",
            Duration::from_secs(2),
            Duration::from_secs(2),
        )
        .await
        .expect("the mock is serving");
        let AgentSelection::External(client) = selection else {
            panic!("a serving Agent is an external client, not a local-start permit");
        };
        (*client, stop, task)
    }

    fn request() -> ContentRequest {
        ContentRequest {
            database_id: DATABASE.into(),
            restore_generation: GENERATION.into(),
            epoch: 1,
            cursor: 2,
            field: "/data/text".into(),
            offset: 0,
            max_bytes: 64 * 1024,
        }
    }

    /// One canonical page, as the host serializes it.
    fn page(text: &str, next: i64, total: i64, more: bool, etag: &str, encoding: &str) -> Value {
        json!({
            "text": text,
            "nextOffset": next,
            "totalBytes": total,
            "hasMore": more,
            "etag": etag,
            "encoding": encoding,
            "databaseId": DATABASE,
            "restoreGeneration": GENERATION,
        })
    }

    /// A text page that covers exactly `text` and ends the feed.
    fn whole(text: &str) -> Value {
        let length = text.len() as i64;
        page(text, length, length, false, "etag-1", "text")
    }

    #[tokio::test]
    async fn a_single_text_page_comes_back_verbatim() {
        let (mut client, stop, task) = serving(vec![whole("hello")]).await;
        let value = read_field(&mut client, "s", "r", request())
            .await
            .expect("one page");
        assert_eq!(value, Value::String("hello".into()));
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_json_page_is_parsed_into_a_value_rather_than_left_as_text() {
        let body = "{\"a\":1}";
        let (mut client, stop, task) = serving(vec![page(
            body,
            body.len() as i64,
            body.len() as i64,
            false,
            "etag-1",
            "json",
        )])
        .await;
        let value = read_field(&mut client, "s", "r", request())
            .await
            .expect("one page");
        assert_eq!(value, json!({"a": 1}));
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn pages_are_joined_until_the_feed_says_it_is_done() {
        let (mut client, stop, task) = serving(vec![
            page("he", 2, 5, true, "etag-1", "text"),
            page("llo", 5, 5, false, "etag-1", "text"),
        ])
        .await;
        let value = read_field(&mut client, "s", "r", request())
            .await
            .expect("two pages");
        assert_eq!(value, Value::String("hello".into()));
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_body_that_changed_between_pages_is_refused_rather_than_mixed() {
        let (mut client, stop, task) = serving(vec![
            page("he", 2, 5, true, "etag-1", "text"),
            page("llo", 5, 5, false, "etag-2", "text"),
        ])
        .await;
        let error = read_field(&mut client, "s", "r", request())
            .await
            .expect_err("two etags");
        assert_eq!(error, "content page changed while reading");
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_field_larger_than_the_materialization_budget_names_the_way_out() {
        let oversized = (MAX_MATERIALIZED_BYTES + 1) as i64;
        let (mut client, stop, task) =
            serving(vec![page("he", 2, oversized, true, "etag-1", "text")]).await;
        let error = read_field(&mut client, "s", "r", request())
            .await
            .expect_err("over budget");
        assert!(error.contains("exceeds materialization budget"), "{error}");
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_body_that_is_not_a_page_is_reported_as_such() {
        let (mut client, stop, task) = serving(vec![json!({"nextOffset": 0})]).await;
        let error = read_field(&mut client, "s", "r", request())
            .await
            .expect_err("no text");
        assert_eq!(error, "invalid content page text");
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_refused_page_surfaces_the_hosts_own_message() {
        // No pages queued: the mock refuses, which is what a host that stopped
        // publishing this body looks like.
        let (mut client, stop, task) = serving(Vec::new()).await;
        let error = read_field(&mut client, "s", "r", request())
            .await
            .expect_err("refused");
        assert_eq!(error, "no content pages left");
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn only_the_kinds_a_decision_needs_are_materialized() {
        assert!(required_inline("user_message"));
        assert!(required_inline("approval_request"));
        assert!(!required_inline("tool_end"));

        // A tool body stays a reference: hydrating it would pull megabytes to
        // show a line the reader has not asked for.
        let (mut client, stop, task) = serving(Vec::new()).await;
        let mut data = json!({"content_refs": {"output": {"field": "/data/output"}}});
        hydrate_required(&mut client, "s", "r", "tool_end", &mut data)
            .await
            .expect("a lazy kind is left alone");
        assert_eq!(data["content_refs"]["output"]["field"], "/data/output");
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_referenced_field_is_replaced_by_its_text_and_the_reference_is_dropped() {
        let (mut client, stop, task) = serving(vec![whole("hello")]).await;
        let mut data = json!({
            "text": "",
            "content_refs": {
                "text": {
                    "databaseId": DATABASE,
                    "restoreGeneration": GENERATION,
                    "epoch": 1,
                    "cursor": 2,
                    "field": "/data/text"
                }
            }
        });
        hydrate_required(&mut client, "s", "r", "user_message", &mut data)
            .await
            .expect("one referenced field");
        assert_eq!(data["text"], "hello");
        assert!(data.get("content_refs").is_none(), "{data}");
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_reference_pointing_at_another_field_is_refused_before_any_read() {
        let (mut client, stop, task) = serving(Vec::new()).await;
        let mut data = json!({
            "text": "",
            "content_refs": {
                "text": {
                    "databaseId": DATABASE,
                    "restoreGeneration": GENERATION,
                    "epoch": 1,
                    "cursor": 2,
                    "field": "/data/somewhere-else"
                }
            }
        });
        let error = hydrate_required(&mut client, "s", "r", "user_message", &mut data)
            .await
            .expect_err("a reference is only allowed to name its own field");
        assert_eq!(error, "event field reference mismatch");
        let _ = stop.send(());
        task.await.unwrap();
    }

    #[test]
    fn a_stream_event_is_only_hydrated_when_it_or_its_snapshot_carries_a_reference() {
        let plain = proto::StreamEvent {
            r#type: "agent_text".into(),
            data: "{\"text\":\"hi\"}".into(),
            ..Default::default()
        };
        assert!(!stream_needs_hydration(&plain));

        let referenced = proto::StreamEvent {
            r#type: "user_message".into(),
            data: "{\"content_refs\":{}}".into(),
            ..Default::default()
        };
        assert!(stream_needs_hydration(&referenced));

        // The envelope itself carries nothing; a folded projection inside it
        // does — that is the form a replayed snapshot arrives in.
        let snapshot = proto::StreamEvent {
            r#type: "agent_end".into(),
            data: "{}".into(),
            snapshot_events: vec![proto::ProjectedRunEvent {
                r#type: "user_message".into(),
                data: "{\"content_refs\":{}}".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(stream_needs_hydration(&snapshot));
    }
}
