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

/// Assemble oversized entries before exposing them to message projection.
/// Backward transport pages may split an exchange; fetch its missing prefix
/// too, since renderers need the user and tool declarations to interpret it.
pub async fn read_page<Entry, F, Fut>(
    mut command: RpcCommand,
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
            let needs_prefix = accumulated
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
}
