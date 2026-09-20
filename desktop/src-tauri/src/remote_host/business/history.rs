use crate::remote::protocol::IncomingCmd;
use crate::remote::services::ReplySink;
use serde_json::{json, Value};

use super::{
    empty_entries_page, entries_vec, missing_session, paginate_events, paginate_items_with_cap,
    paginate_messages, prepare_backward_entries_page_with_cap, reply, DEFAULT_MESSAGE_PAGE_LIMIT,
};

/// The cursor meaning "the page at the end of the session". Clients open a
/// conversation with `Number.MAX_SAFE_INTEGER`; anything at or above it is a
/// first paint, and anything below it is a scrolled-up page.
const NEWEST_PAGE_CURSOR: i64 = 9_007_199_254_740_991;

fn is_newest_page(before: Option<i64>) -> bool {
    // An absent cursor is the forward-paging path, which carries its own bounds.
    before.is_none_or(|before| before >= NEWEST_PAGE_CURSOR)
}

pub(super) async fn execute(cmd: &IncomingCmd, sink: &dyn ReplySink) {
    match cmd.cmd_type.as_str() {
        "get_messages" => {
            // Serve history from the agent (source of truth for all sessions).
            // The GUI store only has message rows for GUI-native threads —
            // TUI/CLI sessions imported as thread stubs would show empty history.
            let offset = cmd.offset.max(0) as usize;
            let limit = if cmd.limit > 0 {
                cmd.limit as usize
            } else {
                DEFAULT_MESSAGE_PAGE_LIMIT
            };
            let messages = match crate::agent_bridge::get_session_messages(cmd.session_id.clone())
                .await
            {
                Ok(data) => super::messages_vec(data),
                Err(agent_err) => {
                    reply(
                        sink,
                        false,
                        Value::Null,
                        Some(&format!(
                            "{agent_err}; conversation history is unavailable while the Agent is offline"
                        )),
                    )
                    .await;
                    return;
                }
            };
            reply(sink, true, paginate_messages(messages, offset, limit), None).await;
        }
        "get_session_entries" => {
            let offset = cmd.offset.max(0) as usize;
            let limit = if cmd.limit > 0 {
                cmd.limit as usize
            } else {
                DEFAULT_MESSAGE_PAGE_LIMIT
            };
            if let Some(before) = cmd.before {
                match crate::agent_bridge::get_session_entries_before(
                    cmd.session_id.clone(),
                    before,
                    limit as i64,
                )
                .await
                {
                    Ok(data) => {
                        // The newest page is a first paint: the user is waiting
                        // for the first screenful, so it pays the byte budget.
                        // A scrolled-up page ("the page before this cursor") gets
                        // whole user exchanges instead — trimming those only
                        // splits the same bytes into more round trips, which is
                        // what the one-second loading indicator shows the user.
                        // Chunked content stays lossless either way.
                        let first_paint = is_newest_page(cmd.before);
                        let page = prepare_backward_entries_page_with_cap(
                            &cmd.session_id,
                            data,
                            !cmd.chunked_read,
                            first_paint,
                        );
                        reply(sink, true, page, None).await;
                    }
                    Err(e) if missing_session(&e) => {
                        reply(sink, true, empty_entries_page(), None).await
                    }
                    Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
                }
                return;
            }
            match crate::agent_bridge::get_session_entries(cmd.session_id.clone()).await {
                Ok(data) => {
                    let entries = entries_vec(data);
                    reply(
                        sink,
                        true,
                        paginate_items_with_cap(
                            entries,
                            offset,
                            limit,
                            "entries",
                            !cmd.chunked_read,
                        ),
                        None,
                    )
                    .await;
                }
                Err(e) if missing_session(&e) => {
                    reply(sink, true, empty_entries_page(), None).await;
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        "get_events_since" => {
            if cmd.prefer_snapshot
                && cmd.chunked_read
                && cmd.since_idx == -1
                && cmd.offset == 0
                && cmd.replay_until_idx.is_none()
            {
                match crate::agent_bridge::get_run_snapshot(
                    cmd.session_id.clone(),
                    cmd.run_id.clone(),
                )
                .await
                {
                    Ok(Some(snapshot)) => {
                        reply(sink, true, snapshot, None).await;
                        return;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        reply(sink, false, Value::Null, Some(&error.to_string())).await;
                        return;
                    }
                }
            }
            let offset = cmd.offset.max(0) as usize;
            let limit = if cmd.limit > 0 {
                cmd.limit as usize
            } else {
                DEFAULT_MESSAGE_PAGE_LIMIT
            };
            let data = if cmd.replay_until_idx.is_some() && offset == 0 {
                crate::agent_bridge::get_events_since_page(
                    cmd.session_id.clone(),
                    cmd.run_id.clone(),
                    cmd.since_idx,
                    limit,
                )
                .await
            } else {
                crate::agent_bridge::get_events_since(
                    cmd.session_id.clone(),
                    cmd.run_id.clone(),
                    cmd.since_idx,
                )
                .await
            };
            match data {
                Ok(mut data) => {
                    let source_watermark = data["events"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|event| event["idx"].as_i64())
                        .max()
                        .unwrap_or(cmd.since_idx)
                        .max(data["projection"]["cursor"].as_i64().unwrap_or(-1));
                    let watermark = cmd.replay_until_idx.unwrap_or(source_watermark);
                    if data["projection"]["cursor"]
                        .as_i64()
                        .is_some_and(|cursor| cursor > watermark)
                    {
                        reply(sink, false, Value::Null, Some("replay_window_changed")).await;
                        return;
                    }
                    if let Some(events) = data["events"].as_array_mut() {
                        events.retain(|event| {
                            event["idx"].as_i64().is_none_or(|idx| idx <= watermark)
                        });
                    }
                    let agent_has_more = data["hasMore"].as_bool().unwrap_or(false);
                    let mut page = paginate_events(data, offset, limit);
                    page["watermark"] = json!(watermark);
                    page["nextSinceIdx"] = page["events"]
                        .as_array()
                        .and_then(|events| events.last())
                        .and_then(|event| event.get("idx"))
                        .cloned()
                        .unwrap_or(json!(cmd.since_idx));
                    let next = page["nextSinceIdx"].as_i64().unwrap_or(cmd.since_idx);
                    page["hasMore"] = json!(
                        next < watermark
                            && (agent_has_more || page["hasMore"].as_bool().unwrap_or(false))
                    );
                    reply(sink, true, page, None).await;
                }
                Err(e) => reply(sink, false, Value::Null, Some(&e.to_string())).await,
            }
        }
        _ => unreachable!("history handler received {}", cmd.cmd_type),
    }
}

#[cfg(test)]
mod tests {
    use super::{is_newest_page, NEWEST_PAGE_CURSOR};

    /// Only the page the user is waiting to see pays the byte budget. A page
    /// asked for as "the one before cursor X" is a scroll-up, where trimming
    /// the same bytes into more pages only adds round trips and loading-indicator
    /// flashes.
    #[test]
    fn only_the_newest_page_is_a_first_paint() {
        // How the mobile opens a conversation.
        assert_eq!(NEWEST_PAGE_CURSOR, 9_007_199_254_740_991);
        assert!(is_newest_page(Some(NEWEST_PAGE_CURSOR)));
        // A cursor at or above the tail is still "from the end".
        assert!(is_newest_page(Some(i64::MAX)));
        // An absent cursor is the forward-paging path, bounded on its own.
        assert!(is_newest_page(None));
        // A real scroll-up cursor is not a first paint. These are the values
        // the phone actually sends (session ordinals).
        for before in [2_173_i64, 1_950, 859, 1, 0] {
            assert!(!is_newest_page(Some(before)), "cursor {before}");
        }
    }
}
