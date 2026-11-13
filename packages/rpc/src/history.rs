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
    /// What this row adds to a page once the page is encoded.
    ///
    /// [`FIRST_PAGE_BYTE_BUDGET`] bounds bytes, so the assembly loop has to be
    /// able to weigh what it is holding. The number is the row's own JSON: a page
    /// is an array of them, and the envelope's brackets and commas are a few
    /// bytes per row against pages measured in hundreds of kilobytes.
    fn page_bytes(&self) -> usize;
}
impl HistoryEntry for crate::payloads::SessionEntryPayload {
    fn entry_id(&self) -> Option<&str> {
        Some(&self.id)
    }
    fn needs_user_prefix(&self) -> bool {
        needs_user_prefix(Some(&self.role), Some(&self.kind), self.checkpoint.as_ref())
    }
    fn page_bytes(&self) -> usize {
        serde_json::to_vec(self).map_or(0, |bytes| bytes.len())
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
    fn page_bytes(&self) -> usize {
        serde_json::to_vec(self).map_or(0, |bytes| bytes.len())
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
///   reply answers; completing the exchange costs one small extra fetch, and the
///   walk that would grow the page to the turn's size is bounded by
///   [`FIRST_PAGE_BYTE_BUDGET`] — a turn too big to complete comes back as its
///   tail, saying so.
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

/// The most one assembled backward page may weigh before the walk that
/// completes its exchange is cut short.
///
/// This is the host's own reply budget, not a second one: `wire_limits`
/// (`BACKWARD_HISTORY_PAGE_BYTES`, 512 KiB) is what a history reply promises to
/// fit, and it keeps that promise by deferring whole exchanges. A turn larger
/// than the budget on its own is one exchange, so there is nothing there to
/// defer — the page leaves whole. Measured on the reported conversation
/// (2026-11-13): one turn of 1,113 rows, 10.35 MB raw and 5.22 MB after the lean
/// trim, pulled in full on every open.
///
/// So the bound has to be applied where the page is *assembled*. Completing an
/// exchange is worth one small extra fetch; it is not worth a turn's whole body.
/// A page that cannot reach its question within this budget is served as the
/// newest rows that fit and says so (`truncated`), which keeps the promise above
/// for any data: no single turn can turn an open into a multi-megabyte transfer.
pub const FIRST_PAGE_BYTE_BUDGET: usize = 512 * 1024;

/// How much one first paint may carry, and how a row is counted against it.
///
/// Two independent bounds, because they pay for different things:
///
/// * `bytes` is the transfer, measured with the caller's **weight** — that is,
///   by how the page will be *sent*. The host weighs a lean page by what the
///   lean trim leaves behind, not by what the Agent handed it: measured
///   2026-11-13 on the reported conversation, one turn was 12.99 MB raw and
///   1.75 MB once the unread bodies were gone (13.5%). Counting the raw bytes
///   kept 33 rows; the same 512 KiB of shipped bytes holds several hundred.
/// * `rows` is the render. A thousand rows of two hundred bytes each are cheap
///   to move and expensive to draw, and "big enough to page" is a different
///   question from "big enough to show".
#[derive(Clone, Copy, Debug)]
pub struct PageBudget {
    /// Bytes as the caller will send them, weighed by its own function.
    pub bytes: usize,
    /// Rows, whatever they weigh.
    pub rows: usize,
}

impl PageBudget {
    /// What a first paint gets: 512 KiB as they are sent, and at most 300 rows.
    ///
    /// Three hundred is a render bound, not a transfer one: a tool-dense run
    /// writes an assistant step and a result per call, and a page of a thousand
    /// of them is a thousand row models to build before the newest screen is on
    /// screen. The rows a reader actually reads in one screenful are far fewer;
    /// the rest is a scroll away, one page per scroll.
    pub const FIRST_PAINT: PageBudget = PageBudget {
        bytes: FIRST_PAGE_BYTE_BUDGET,
        rows: 300,
    };
}

/// Assemble oversized entries before exposing them to message projection.
///
/// A backward transport page may split an exchange; the missing prefix is
/// fetched too (`ExchangeStart::Complete`), since renderers need the user and
/// tool declarations to interpret it. A page that is allowed to begin anywhere
/// (`ExchangeStart::Anywhere`) is returned as the agent cut it.
///
/// Not an `async fn`, and that is the point: the only thing it adds to
/// [`read_page_from`] is a constant, and an `async fn` would pay for that
/// constant with a second state machine. That is not free in the one caller
/// that matters — the desktop's history read is the deepest `await` chain in the
/// host (`remote_host::business` → `history::execute` → the page read → the
/// agent call), and it runs on a worker thread with the platform's default stack.
/// Measured on the `remote::commands::bridge_tests` group, which reaches it: the
/// redundant nesting cost **78 KiB** of the 2 MiB a worker gets in a debug build
/// (1970 KiB against 1892 KiB when the wrapper is transparent), which is what
/// took `Rust Windows test / host-remote` over the edge (MSVC debug frames are
/// wider than arm64 ones). Keep this a plain `fn`: there is nothing to await on
/// this side of the call.
pub fn read_page<Entry, F, Fut>(
    command: RpcCommand,
    execute: F,
) -> impl Future<Output = Result<SessionEntriesPage<Entry>, String>>
where
    Entry: HistoryEntry,
    F: FnMut(RpcCommand) -> Fut,
    Fut: Future<Output = Result<SessionEntriesPage<Entry>, String>>,
{
    read_page_from(command, ExchangeStart::Complete, execute)
}

/// [`read_page`], naming what a page may do to the exchange it starts in.
pub async fn read_page_from<Entry, F, Fut>(
    command: RpcCommand,
    start: ExchangeStart,
    execute: F,
) -> Result<SessionEntriesPage<Entry>, String>
where
    Entry: HistoryEntry,
    F: FnMut(RpcCommand) -> Fut,
    Fut: Future<Output = Result<SessionEntriesPage<Entry>, String>>,
{
    read_page_with(
        command,
        start,
        PageBudget::FIRST_PAINT,
        |entry: &Entry| entry.page_bytes(),
        execute,
    )
    .await
}

/// [`read_page_from`], naming the budget and how a row is weighed against it.
///
/// The weight is the caller's, because the caller is the one that knows what it
/// will send: the host weighs a page the way its lean lane will ship it, the
/// terminal weighs the row itself. A test drives a small budget to pin the
/// truncation; production callers go through [`read_page_from`].
pub async fn read_page_with<Entry, F, Fut, W>(
    mut command: RpcCommand,
    start: ExchangeStart,
    budget: PageBudget,
    weigh: W,
    mut execute: F,
) -> Result<SessionEntriesPage<Entry>, String>
where
    Entry: HistoryEntry,
    W: Fn(&Entry) -> usize,
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
                // Ask for the next older slice only while the page so far is
                // still a page: completing an exchange is worth one small extra
                // fetch, not a turn's whole body. When the turn is bigger than
                // the budget on its own, no number of further slices can reach
                // its question, and the page is served as that turn's tail.
                let dropped = trim_to_budget(&mut accumulated, budget, &weigh);
                if dropped > 0 {
                    page.entries = accumulated;
                    // What left the page is reachable, not lost: the cursor names
                    // the first row this page still carries.
                    page.next_offset += dropped as i64;
                    page.has_more = true;
                    page.truncated = true;
                    return Ok(page);
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

/// Drop the oldest rows of a page until it fits `budget`, and say how many left.
///
/// At least one row always stays. A single row can be larger than the budget on
/// its own — an entry is the smallest unit a page can carry, and the body that
/// makes it big is what the fragment path and the client's on-demand fetch are
/// for — so the promise is "a page is the newest rows that fit, or one row",
/// never "an empty page".
fn trim_to_budget<Entry, W>(entries: &mut Vec<Entry>, budget: PageBudget, weigh: &W) -> usize
where
    W: Fn(&Entry) -> usize,
{
    let weights: Vec<usize> = entries.iter().map(weigh).collect();
    let mut total: usize = weights.iter().sum();
    let mut dropped = 0;
    while dropped + 1 < entries.len()
        && (total > budget.bytes || entries.len() - dropped > budget.rows)
    {
        total -= weights[dropped];
        dropped += 1;
    }
    if dropped > 0 {
        entries.drain(..dropped);
    }
    dropped
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
            total: None,
            truncated: false,
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
            total: None,
            fragment,
            truncated: false,
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
        scripted_with_budget(
            command,
            start,
            PageBudget::FIRST_PAINT,
            |entry: &Entry| entry.page_bytes(),
            pages,
        )
        .await
    }

    /// [`scripted_from`], naming the budget and how a row is weighed against it.
    async fn scripted_with_budget<Entry>(
        command: RpcCommand,
        start: ExchangeStart,
        budget: PageBudget,
        weigh: fn(&Entry) -> usize,
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
            read_page_with::<Entry, _, _, _>(command, start, budget, weigh, &mut execute).await
        };
        (result, seen.into_inner())
    }

    /// The assertions that follow are written against this number, so changing
    /// it has to be a failing test rather than a quiet widening: a budget that is
    /// "no bound" restores the transfer they exist to prevent.
    #[test]
    fn the_first_page_budget_is_the_host_reply_budget() {
        assert_eq!(FIRST_PAGE_BYTE_BUDGET, 512 * 1024);
    }

    /// Rows shaped like the reported turn's: an assistant step per row, each
    /// carrying a body worth tens of kilobytes.
    fn turn_rows(rows: usize, each: usize) -> Vec<Value> {
        (0..rows)
            .map(|index| {
                json!({
                    "id": format!("a{index}"),
                    "kind": "assistant",
                    "role": "assistant",
                    "blocks": [{"kind": "text", "text": "x".repeat(each)}],
                })
            })
            .collect()
    }

    /// A turn too big to complete is served as its tail, not as itself.
    ///
    /// This is the reported conversation (2026-11-13): one turn of 1,113 rows
    /// and 10.35 MB. Completing it means walking back until the page starts at
    /// its question, which for that turn is the whole thing — the pages here are
    /// rows of it, and the walk must stop instead of asking for the rest.
    ///
    /// Removing the tail (returning the accumulated page) makes the `<=
    /// FIRST_PAGE_BYTE_BUDGET` line below fail, which is the point of writing it
    /// against the constant.
    #[tokio::test]
    async fn a_turn_larger_than_the_budget_comes_back_as_its_tail() {
        let rows = turn_rows(30, 40_000);
        let whole: usize = rows.iter().map(HistoryEntry::page_bytes).sum();
        assert!(
            whole > FIRST_PAGE_BYTE_BUDGET,
            "the fixture has to be the case this bound is for: {whole} bytes"
        );
        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };
        let (result, seen) =
            scripted::<Value>(command, vec![page(None, rows.clone(), true, 1_000)]).await;

        let served = result.expect("a truncated page is a page");
        assert_eq!(
            seen.len(),
            1,
            "the walk stops at the budget instead of fetching the rest of the turn"
        );
        assert!(served.truncated, "the page says what it left behind");
        let bytes: usize = served.entries.iter().map(HistoryEntry::page_bytes).sum();
        assert!(
            bytes <= FIRST_PAGE_BYTE_BUDGET,
            "a first page is bounded whatever the turn weighs: {bytes} bytes"
        );
        // The newest rows are the ones kept, and the cursor names the first of
        // them: what left the page is still reachable on the next pull.
        let kept = served.entries.len();
        assert_eq!(
            served.entries.last().map(HistoryEntry::entry_id),
            rows.last().map(HistoryEntry::entry_id),
            "the tail is the newest row"
        );
        assert_eq!(served.next_offset, 1_000 + (rows.len() - kept) as i64);
        assert!(served.has_more, "the rest of the turn is behind the cursor");
    }

    /// An exchange that fits is still completed to its question.
    ///
    /// The budget bounds the walk; it does not change what a page is for readers
    /// the walk can finish. This is the ordinary conversation: one small older
    /// slice, and the page begins at the prompt it answers.
    #[tokio::test]
    async fn an_exchange_that_fits_is_still_completed_to_its_question() {
        let answer = || json!({"id": "a1", "kind": "assistant", "role": "assistant"});
        let question = || json!({"id": "u0", "kind": "user", "role": "user"});
        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };
        let (result, seen) = scripted::<Value>(
            command,
            vec![
                page(None, vec![answer()], true, 10),
                page(None, vec![question(), answer()], true, 3),
            ],
        )
        .await;

        let served = result.expect("page");
        assert_eq!(seen.len(), 2, "one completion fetch, then the question");
        assert!(!served.truncated, "nothing was left behind");
        assert_eq!(served.entries.len(), 3);
        assert_eq!(
            served.entries[0],
            question(),
            "the page begins at the question"
        );
        assert_eq!(served.next_offset, 3);
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

    /// A first paint is bounded by rows as well as bytes.
    ///
    /// A thousand rows of a few bytes each are nowhere near the byte budget and
    /// are still a thousand row models to build before the newest screen is on
    /// screen, so the page stops at the row bound and says what it left behind.
    #[tokio::test]
    async fn a_first_paint_is_bounded_by_rows_as_well_as_bytes() {
        let rows = turn_rows(1_000, 10);
        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };
        let (result, _) =
            scripted::<Value>(command, vec![page(None, rows.clone(), true, 5_000)]).await;
        let served = result.expect("page");
        assert!(served.truncated, "the page says what it left behind");
        assert_eq!(
            served.entries.len(),
            PageBudget::FIRST_PAINT.rows,
            "the row bound is what stopped it, not the bytes"
        );
        assert_eq!(
            served.next_offset,
            5_000 + (rows.len() - PageBudget::FIRST_PAINT.rows) as i64
        );
    }

    /// The budget is spent on what will be sent, not on what was read.
    ///
    /// The host weighs a lean page by what its trim leaves; the same bytes then
    /// buy several times the rows a raw weight buys. This is the property the
    /// two weights differ for — if the caller's weight were ignored, the two
    /// pages would be identical.
    #[tokio::test]
    async fn a_cheaper_weight_fits_more_rows_in_the_same_budget() {
        let rows = turn_rows(400, 4_000);
        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };
        let (raw, _) = scripted_with_budget(
            command.clone(),
            ExchangeStart::Complete,
            PageBudget::FIRST_PAINT,
            |entry: &Value| entry.page_bytes(),
            vec![page(None, rows.clone(), true, 1_000)],
        )
        .await;
        let raw = raw.expect("page");
        // The lean lane keeps the row's identity and drops the bodies, so a row
        // weighs about what its id and kind do.
        let (lean, _) = scripted_with_budget(
            command,
            ExchangeStart::Complete,
            PageBudget::FIRST_PAINT,
            |entry: &Value| entry["blocks"][0]["text"].as_str().map_or(80, |_| 80),
            vec![page(None, rows.clone(), true, 1_000)],
        )
        .await;
        let lean = lean.expect("page");
        assert!(
            lean.entries.len() > raw.entries.len() * 2,
            "the same 512 KiB of sent bytes holds far more rows than the raw page did: \
             {} against {}",
            lean.entries.len(),
            raw.entries.len()
        );
        assert!(
            lean.entries.len() <= PageBudget::FIRST_PAINT.rows,
            "and the row bound still applies: {}",
            lean.entries.len()
        );
    }
}
