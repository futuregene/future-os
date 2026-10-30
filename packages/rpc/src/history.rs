//! Lossless history paging shared by desktop and terminal clients.
use crate::payloads::SessionEntriesPage;
use crate::proto::RpcCommand;
use sha2::{Digest, Sha256};
use std::future::Future;

/// Typed desktop entries and legacy terminal JSON share pagination, while the
/// terminal retains unknown fields instead of tightening its wire contract.
pub trait HistoryEntry: serde::de::DeserializeOwned {
    fn entry_id(&self) -> Option<&str>;
    fn needs_user_prefix(&self) -> bool;
}
impl HistoryEntry for crate::payloads::SessionEntryPayload {
    fn entry_id(&self) -> Option<&str> {
        Some(&self.id)
    }
    fn needs_user_prefix(&self) -> bool {
        needs_user_prefix(Some(&self.role), Some(&self.kind), self.checkpoint.as_ref())
    }
}
impl HistoryEntry for serde_json::Value {
    fn entry_id(&self) -> Option<&str> {
        self.get("id").and_then(Self::as_str)
    }
    fn needs_user_prefix(&self) -> bool {
        needs_user_prefix(
            self.get("role").and_then(Self::as_str),
            self.get("kind").and_then(Self::as_str),
            self.get("checkpoint"),
        )
    }
}

fn needs_user_prefix(
    role: Option<&str>,
    kind: Option<&str>,
    checkpoint: Option<&serde_json::Value>,
) -> bool {
    if matches!(role, Some("assistant" | "tool")) {
        return true;
    }
    // An automatic in-turn checkpoint does not start a standalone exchange.
    kind == Some("compaction")
        && checkpoint.is_some_and(|checkpoint| {
            checkpoint["schemaVersion"].as_i64().unwrap_or(0) >= 2
                && checkpoint["phase"] != "standalone"
                && (matches!(checkpoint["phase"].as_str(), Some("pre_turn" | "mid_turn"))
                    || checkpoint["trigger"] != "manual")
        })
}

/// Where a backward page may begin inside the exchange it lands in.
///
/// **The two modes are a deliberate divergence, not an oversight — do not
/// "unify" them.** One paging call serves clients whose first page means
/// different things:
///
/// * [`ExchangeStart::Complete`] — the host's history read (`host::agent_bridge`)
///   behind the desktop and mobile apps. A page there is a slice of a
///   conversation being read front to back, so it begins at the question its
///   reply answers; completing the exchange costs one small extra fetch.
/// * [`ExchangeStart::Anywhere`] — the terminal's conversation switch
///   (`GrpcClient::session_tail_page`). That first page is not a slice to read
///   but a *tail to look at*: the reader wants the newest rows as fast as the
///   agent can cut them, and the agent bounds a page by rows and bytes now (150
///   rows / 2 MB for a first page). Completing the exchange would fetch the whole
///   turn back — measured at 541 rows, 1.69 MB and 629 ms on this store, which is
///   exactly the page the ceiling exists to prevent.
///
/// Which way the cost lands if someone makes them agree: giving the terminal
/// `Complete` restores that whole-turn first page and its 629 ms; giving the host
/// `Anywhere` starts its pages mid-turn, with no question above the reply the
/// reader lands on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExchangeStart {
    /// Fetch the exchange's missing prefix too, so the page begins at the
    /// question its reply answers. Used by a caller reading a conversation front
    /// to back.
    Complete,
    /// Accept a page that begins in the middle of an exchange. Used by a
    /// conversation's *first* page, which is bounded by rows and bytes and must
    /// not be grown back to the turn it lands in.
    Anywhere,
}

/// Assemble oversized entries before exposing them to message projection.
///
/// A backward transport page may split an exchange; the missing prefix is
/// fetched too (`ExchangeStart::Complete`), since renderers need the user and
/// tool declarations to interpret it. A page that is allowed to begin anywhere
/// (`ExchangeStart::Anywhere`) is returned as the agent cut it.
pub async fn read_page<Entry, F, Fut>(
    command: RpcCommand,
    execute: F,
) -> Result<SessionEntriesPage<Entry>, String>
where
    Entry: HistoryEntry,
    F: FnMut(RpcCommand) -> Fut,
    Fut: Future<Output = Result<SessionEntriesPage<Entry>, String>>,
{
    read_page_from(command, ExchangeStart::Complete, execute).await
}

/// [`read_page`], naming what a page may do to the exchange it starts in.
pub async fn read_page_from<Entry, F, Fut>(
    mut command: RpcCommand,
    start: ExchangeStart,
    mut execute: F,
) -> Result<SessionEntriesPage<Entry>, String>
where
    Entry: HistoryEntry,
    F: FnMut(RpcCommand) -> Fut,
    Fut: Future<Output = Result<SessionEntriesPage<Entry>, String>>,
{
    let backward = command.before.is_some();
    let mut accumulated = Vec::new();
    loop {
        let mut page = execute(command.clone()).await?;
        if let Some(first) = page.fragment.take() {
            if first.offset != 0 || !page.entries.is_empty() {
                return Err("invalid initial history fragment".into());
            }
            let mut text = first.json.clone();
            loop {
                if text.len() as i64 > first.total_bytes || first.json.is_empty() {
                    return Err("invalid history fragment size".into());
                }
                if text.len() as i64 == first.total_bytes {
                    break;
                }
                command.fragment_offset = Some(text.len() as i64);
                let continuation = execute(command.clone()).await?;
                let fragment = continuation
                    .fragment
                    .ok_or("history changed during fragmented read")?;
                if !continuation.entries.is_empty()
                    || fragment.json.is_empty()
                    || fragment.offset != text.len() as i64
                    || fragment.ordinal != first.ordinal
                    || fragment.entry_id != first.entry_id
                    || fragment.total_bytes != first.total_bytes
                    || fragment.sha256 != first.sha256
                    || continuation.has_more != page.has_more
                    || continuation.next_offset != page.next_offset
                {
                    return Err("history changed during fragmented read".into());
                }
                text.push_str(&fragment.json);
            }
            if format!("{:x}", Sha256::digest(text.as_bytes())) != first.sha256 {
                return Err("history fragment checksum mismatch".into());
            }
            let entry: Entry = serde_json::from_str(&text)
                .map_err(|e| format!("invalid assembled history entry: {e}"))?;
            if entry.entry_id() != Some(first.entry_id.as_str()) {
                return Err("history fragment identity mismatch".into());
            }
            page.entries.push(entry);
            command.fragment_offset = None;
        } else if command.fragment_offset.is_some() {
            return Err("missing history fragment".into());
        }
        if backward {
            page.entries.append(&mut accumulated);
            accumulated = page.entries;
            let needs_prefix = start == ExchangeStart::Complete
                && accumulated
                    .first()
                    .is_some_and(HistoryEntry::needs_user_prefix);
            if page.has_more && needs_prefix {
                if accumulated.is_empty() || page.next_offset >= command.before.unwrap_or(i64::MAX)
                {
                    return Err("history returned a non-advancing backward cursor".into());
                }
                command.before = Some(page.next_offset);
                command.limit = Some(1);
                continue;
            }
            page.entries = accumulated;
        }
        return Ok(page);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payloads::{HistoryEntryFragment, SessionEntryPayload};
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    fn fragment(offset: i64, text: &str, full: &str) -> SessionEntriesPage {
        SessionEntriesPage {
            entries: Vec::new(),
            has_more: false,
            next_offset: 1,
            fragment: Some(HistoryEntryFragment {
                entry_id: "large".into(),
                ordinal: 0,
                offset,
                total_bytes: full.len() as i64,
                json: text.into(),
                sha256: format!("{:x}", Sha256::digest(full.as_bytes())),
            }),
        }
    }

    #[tokio::test]
    async fn missing_changed_or_corrupted_fragments_are_never_returned_as_history() {
        let full = serde_json::to_string(&SessionEntryPayload {
            id: "large".into(),
            ..Default::default()
        })
        .unwrap();
        let split = full.len() / 2;
        for corruption in [
            "offset", "checksum", "identity", "missing", "empty", "bytes",
        ] {
            let mut pages = std::collections::VecDeque::from([
                fragment(0, &full[..split], &full),
                fragment(split as i64, &full[split..], &full),
            ]);
            let last = pages.back_mut().unwrap();
            match corruption {
                "offset" => last.fragment.as_mut().unwrap().offset += 1,
                "checksum" => last.fragment.as_mut().unwrap().sha256 = "different".into(),
                "identity" => last.fragment.as_mut().unwrap().entry_id = "different".into(),
                "missing" => last.fragment = None,
                "empty" => last.fragment.as_mut().unwrap().json.clear(),
                "bytes" => last.fragment.as_mut().unwrap().json = "x".repeat(full.len() - split),
                _ => unreachable!(),
            }
            let result = read_page(
                RpcCommand {
                    offset: Some(0),
                    ..Default::default()
                },
                |_| {
                    std::future::ready(Ok(pages
                        .pop_front()
                        .expect("must stop on invalid fragment")))
                },
            )
            .await;
            assert!(result.is_err(), "{corruption} must fail closed");
        }
    }

    fn frag(offset: i64, text: &str, full: &str, entry_id: &str) -> HistoryEntryFragment {
        HistoryEntryFragment {
            entry_id: entry_id.into(),
            ordinal: 0,
            offset,
            total_bytes: full.len() as i64,
            json: text.into(),
            sha256: format!("{:x}", Sha256::digest(full.as_bytes())),
        }
    }

    fn page<Entry>(
        fragment: Option<HistoryEntryFragment>,
        entries: Vec<Entry>,
        has_more: bool,
        next_offset: i64,
    ) -> SessionEntriesPage<Entry> {
        SessionEntriesPage {
            entries,
            has_more,
            next_offset,
            fragment,
        }
    }

    /// Drive `read_page` from a scripted list of pages, one per call, and report
    /// both the outcome and the commands the executor actually received.
    async fn scripted<Entry>(
        command: RpcCommand,
        pages: Vec<SessionEntriesPage<Entry>>,
    ) -> (Result<SessionEntriesPage<Entry>, String>, Vec<RpcCommand>)
    where
        Entry: HistoryEntry + DeserializeOwned,
    {
        scripted_from(command, ExchangeStart::Complete, pages).await
    }

    /// [`scripted`], naming what the read may do to the exchange it starts in.
    async fn scripted_from<Entry>(
        command: RpcCommand,
        start: ExchangeStart,
        pages: Vec<SessionEntriesPage<Entry>>,
    ) -> (Result<SessionEntriesPage<Entry>, String>, Vec<RpcCommand>)
    where
        Entry: HistoryEntry + DeserializeOwned,
    {
        let mut pages = std::collections::VecDeque::from(pages);
        let seen = std::cell::RefCell::new(Vec::new());
        let result = {
            let mut execute = |command: RpcCommand| {
                seen.borrow_mut().push(command);
                std::future::ready(Ok(pages.pop_front().expect("scripted page")))
            };
            read_page_from::<Entry, _, _>(command, start, &mut execute).await
        };
        (result, seen.into_inner())
    }

    /// A page that is allowed to begin anywhere is returned as the agent cut it.
    ///
    /// The completion fetch (`limit = 1` for the exchange's missing prefix) is
    /// what would undo a row-bounded page: a turn with hundreds of rows in it can
    /// never be completed inside a bounded page, so the read would fetch the
    /// whole turn back one exchange at a time.
    #[tokio::test]
    async fn a_page_that_may_begin_anywhere_is_not_completed() {
        let partial = || {
            page::<Value>(
                None,
                vec![json!({"id": "a1", "role": "assistant"})],
                true,
                7,
            )
        };
        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };

        let (result, seen) =
            scripted_from::<Value>(command.clone(), ExchangeStart::Anywhere, vec![partial()]).await;
        let cut = result.expect("a partial page is a page");
        assert_eq!(seen.len(), 1, "one fetch, no prefix: {:?}", seen.len());
        assert_eq!(cut.entries.len(), 1, "the rows the agent cut");
        assert!(cut.has_more, "and the way up is intact");

        // The default completes the same page: one more fetch, asking for the
        // exchange's prefix.
        let (result, seen) = scripted::<Value>(
            command,
            vec![partial(), page(None, Vec::<Value>::new(), false, 0)],
        )
        .await;
        assert_eq!(seen.len(), 2, "the prefix was fetched: {:?}", seen.len());
        assert_eq!(seen[1].limit, Some(1), "one exchange at a time");
        assert!(result.is_ok());
    }

    /// A two-chunk fragmented read of `json`, whose first fragment claims
    /// `entry_id`.
    async fn assemble<Entry>(
        entry_id: &str,
        json: &str,
    ) -> Result<SessionEntriesPage<Entry>, String>
    where
        Entry: HistoryEntry + DeserializeOwned,
    {
        let first_len = json.len() / 2 + 1;
        let (first, second) = json.split_at(first_len);
        scripted::<Entry>(
            RpcCommand {
                offset: Some(0),
                ..Default::default()
            },
            vec![
                page(Some(frag(0, first, json, entry_id)), vec![], false, 9),
                page(
                    Some(frag(first_len as i64, second, json, entry_id)),
                    vec![],
                    false,
                    9,
                ),
            ],
        )
        .await
        .0
    }

    /// The typed desktop shape and the legacy terminal shape must both assemble
    /// an oversized entry, and both must report the assembled entry's own id as
    /// the page's identity.
    #[tokio::test]
    async fn oversized_entries_are_assembled_for_both_entry_shapes() {
        let typed = serde_json::to_string(&SessionEntryPayload {
            id: "large".into(),
            role: "assistant".into(),
            ..Default::default()
        })
        .unwrap();
        let assembled = assemble::<SessionEntryPayload>("large", &typed)
            .await
            .expect("typed entry assembles");
        assert_eq!(assembled.entries.len(), 1);
        assert_eq!(assembled.entries[0].id, "large");
        assert_eq!(assembled.entries[0].role, "assistant");
        assert!(assembled.fragment.is_none(), "a fragment is not a page");

        let legacy = serde_json::json!({"id": "legacy-entry", "role": "assistant"}).to_string();
        let assembled = assemble::<Value>("legacy-entry", &legacy)
            .await
            .expect("legacy entry assembles");
        assert_eq!(assembled.entries.len(), 1);
        assert_eq!(assembled.entries[0]["id"], "legacy-entry");
    }

    /// Every fragment agrees on the entry id, so only the assembled document can
    /// reveal that the server reassembled a different entry (an overwritten
    /// session, not a torn page).
    #[tokio::test]
    async fn a_reassembled_entry_with_a_different_identity_is_refused() {
        let error = assemble::<Value>("renamed", &serde_json::json!({"id": "actual"}).to_string())
            .await
            .err()
            .expect("identity mismatch must fail closed");
        assert!(error.contains("identity mismatch"), "{error}");
    }

    /// The first fragment defines the whole entry: a non-zero offset or a page
    /// that already carries entries means the server is not answering the read
    /// that was issued.
    #[tokio::test]
    async fn a_first_fragment_that_is_not_the_entry_start_is_refused() {
        let full = r#"{"id":"large"}"#;
        for scripted_page in [
            page(
                Some(frag(4, full, full, "large")),
                Vec::<Value>::new(),
                false,
                9,
            ),
            page(
                Some(frag(0, full, full, "large")),
                vec![serde_json::json!({"id": "large"})],
                false,
                9,
            ),
        ] {
            let (result, seen) = scripted::<Value>(
                RpcCommand {
                    offset: Some(0),
                    ..Default::default()
                },
                vec![scripted_page],
            )
            .await;
            let error = result
                .err()
                .expect("an invalid first fragment must fail closed");
            assert!(
                error.contains("invalid initial history fragment"),
                "{error}"
            );
            assert_eq!(seen.len(), 1, "the reader must stop instead of paging on");
        }
    }

    /// A fragment that lies about its size (or is empty) can never be trusted to
    /// terminate the read.
    #[tokio::test]
    async fn a_fragment_that_contradicts_its_declared_size_is_refused() {
        for fragment in [
            frag(0, &"x".repeat(20), "short", "large"),
            frag(0, "", r#"{"id":"large"}"#, "large"),
        ] {
            let (result, _) = scripted::<Value>(
                RpcCommand {
                    offset: Some(0),
                    ..Default::default()
                },
                vec![page(Some(fragment), Vec::new(), false, 9)],
            )
            .await;
            let error = result
                .err()
                .expect("an impossible fragment size must fail closed");
            assert!(error.contains("invalid history fragment size"), "{error}");
        }
    }

    /// A continuation the reader asked for as a fragment must be a fragment.
    #[tokio::test]
    async fn a_continuation_without_the_requested_fragment_is_refused() {
        let (result, seen) = scripted::<Value>(
            RpcCommand {
                offset: Some(0),
                fragment_offset: Some(8),
                ..Default::default()
            },
            vec![page(None, Vec::new(), false, 9)],
        )
        .await;
        let error = result.err().expect("a missing fragment must fail closed");
        assert!(error.contains("missing history fragment"), "{error}");
        assert_eq!(seen.len(), 1);
    }

    /// Backward paging must fetch the missing user prefix: an exchange whose
    /// first visible entry is an assistant answer is incomplete on its own.
    /// Both entry shapes go through the same rule.
    #[tokio::test]
    async fn backward_paging_extends_the_page_with_the_missing_user_prefix() {
        let command = RpcCommand {
            before: Some(10),
            limit: Some(5),
            ..Default::default()
        };

        let (typed, seen) = scripted::<SessionEntryPayload>(
            command.clone(),
            vec![
                page(
                    None,
                    vec![SessionEntryPayload {
                        id: "a1".into(),
                        role: "assistant".into(),
                        ..Default::default()
                    }],
                    true,
                    3,
                ),
                page(
                    None,
                    vec![SessionEntryPayload {
                        id: "u1".into(),
                        role: "user".into(),
                        ..Default::default()
                    }],
                    false,
                    2,
                ),
            ],
        )
        .await;
        let typed = typed.expect("the prefix must be fetched");
        let ids: Vec<&str> = typed.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["u1", "a1"], "oldest first, prefix included");
        assert_eq!(
            seen[1],
            RpcCommand {
                before: Some(3),
                limit: Some(1),
                ..Default::default()
            },
            "the follow-up asks for exactly one more entry before the page"
        );

        let (legacy, _) = scripted::<Value>(
            command,
            vec![
                page(
                    None,
                    vec![serde_json::json!({"id": "a2", "role": "assistant"})],
                    true,
                    3,
                ),
                page(
                    None,
                    vec![serde_json::json!({"id": "u2", "role": "user"})],
                    false,
                    2,
                ),
            ],
        )
        .await;
        let legacy = legacy.expect("the prefix must be fetched");
        assert_eq!(legacy.entries[0]["id"], "u2");
        assert_eq!(legacy.entries[1]["id"], "a2");
    }

    /// A follow-up cursor that does not move would page forever.
    #[tokio::test]
    async fn a_backward_cursor_that_does_not_advance_is_refused() {
        let (result, seen) = scripted::<SessionEntryPayload>(
            RpcCommand {
                before: Some(10),
                ..Default::default()
            },
            vec![page(
                None,
                vec![SessionEntryPayload {
                    id: "a1".into(),
                    role: "assistant".into(),
                    ..Default::default()
                }],
                true,
                10,
            )],
        )
        .await;
        let error = result.err().expect("a stalled cursor must fail closed");
        assert!(error.contains("non-advancing backward cursor"), "{error}");
        assert_eq!(seen.len(), 1);
    }

    /// The exchange rule: only an assistant/tool entry opens a standalone
    /// exchange, and an automatic in-turn checkpoint does not.
    #[test]
    fn only_a_standalone_exchange_needs_a_user_prefix() {
        let checkpoint = |value: serde_json::Value| {
            needs_user_prefix(Some("user"), Some("compaction"), Some(&value))
        };
        assert!(needs_user_prefix(Some("assistant"), Some("message"), None));
        assert!(needs_user_prefix(Some("tool"), Some("message"), None));
        assert!(!needs_user_prefix(Some("user"), Some("message"), None));
        assert!(!needs_user_prefix(None, None, None));
        // Not a compaction: the checkpoint is irrelevant.
        assert!(!needs_user_prefix(
            Some("user"),
            Some("message"),
            Some(&serde_json::json!({"schemaVersion": 2, "phase": "pre_turn"}))
        ));
        // Legacy checkpoints (schema 1) have no automatic phase to reason about.
        assert!(!checkpoint(serde_json::json!({
            "schemaVersion": 1,
            "phase": "pre_turn"
        })));
        // A standalone compaction IS a user-visible exchange.
        assert!(!checkpoint(serde_json::json!({
            "schemaVersion": 2,
            "phase": "standalone"
        })));
        assert!(checkpoint(serde_json::json!({
            "schemaVersion": 2,
            "phase": "pre_turn"
        })));
        assert!(checkpoint(serde_json::json!({
            "schemaVersion": 2,
            "phase": "mid_turn"
        })));
        // Any other phase is automatic unless a manual compaction asked for it.
        assert!(checkpoint(serde_json::json!({
            "schemaVersion": 2,
            "phase": "post_turn",
            "trigger": "auto"
        })));
        assert!(!checkpoint(serde_json::json!({
            "schemaVersion": 2,
            "phase": "post_turn",
            "trigger": "manual"
        })));
    }
}
