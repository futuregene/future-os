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

/// How much history one completed backward page may be assembled from.
///
/// The page itself is bounded by its budget; this bounds the *walk* that finds
/// the page's question. A turn taller than the budget has to be read up to its
/// question before the page can be cut from it — the alternative is serving the
/// rows above an unseen question, which is the blank conversation this walk
/// exists for. Most turns reach their question in one or two slices; this is the
/// backstop that keeps a pathological single turn from pulling its whole body
/// back on every open.
///
/// It is a *first paint's* backstop, so it only applies to a bounded budget: a
/// caller that asked for the exchange itself ([`PageBudget::UNBOUNDED`], which
/// [`read_page`] passes) has no screen to keep cheap, and stopping here would
/// serve it the tail of what it asked for — the row cut below then has no
/// question to land on and drops the head of the turn. See `walk_limit`.
const COMPLETION_WALK_BYTES: usize = 8 * 1024 * 1024;

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
    /// Rows, whatever they weigh. The bound yields to the page's question: a
    /// first page that would have to begin mid-run to stay inside it is served
    /// longer instead (see `trim_to_budget`), because a page without a question
    /// projects to nothing at all.
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
    ///
    /// The *byte* bound is what a reply has to fit in, and it is respected by
    /// dropping whole older exchanges. One exchange is indivisible, so a turn
    /// larger than these bytes is served whole — the page is then as large as
    /// the turn it lands in, which is the case the reply lane chunks. The row
    /// bound is only a bound on what fits, and a page may exceed it rather than
    /// lose the question that makes it readable.
    pub const FIRST_PAINT: PageBudget = PageBudget {
        bytes: FIRST_PAGE_BYTE_BUDGET,
        rows: 300,
    };

    /// No bound: the page is whatever completing its exchange produced.
    ///
    /// The bound above is a *first paint's* budget, and a caller that never
    /// declared a first paint must not be served one: the terminal's history
    /// read and the CLI's page read walk the same helper without a screen to
    /// paint, and cutting their page at 300 rows would hand them a turn's tail
    /// where they asked for the turn. Bounding is therefore something a caller
    /// opts into ([`PageBudget::FIRST_PAINT`] at the host's history read and at
    /// the terminal's first page), not something the shared helper applies to
    /// everyone.
    pub const UNBOUNDED: PageBudget = PageBudget {
        bytes: usize::MAX,
        rows: usize::MAX,
    };

    /// Whether this budget bounds nothing, as [`PageBudget::UNBOUNDED`] does.
    ///
    /// A budget that bounds nothing bounds neither the page nor the walk that
    /// completes it: the two are one decision, and a walk stopped at
    /// [`COMPLETION_WALK_BYTES`] under an unbounded budget would hand back the
    /// tail of the exchange the caller asked for.
    fn is_unbounded(self) -> bool {
        self.bytes == usize::MAX && self.rows == usize::MAX
    }

    /// How far the walk to a page's question may read, under this budget.
    ///
    /// A bounded first paint gets the backstop; an unbounded reader gets the
    /// whole exchange, because that is what it asked for.
    fn walk_limit(self) -> usize {
        if self.is_unbounded() {
            usize::MAX
        } else {
            COMPLETION_WALK_BYTES
        }
    }
}

/// Assemble oversized entries before exposing them to message projection.
///
/// A backward transport page may split an exchange; the missing prefix is
/// fetched too (`ExchangeStart::Complete`), since renderers need the user and
/// tool declarations to interpret it. A page that is allowed to begin anywhere
/// (`ExchangeStart::Anywhere`) is returned as the agent cut it.
///
/// Not an `async fn`, and that is the point: it only forwards to
/// [`read_page_with`], and an `async fn` would pay for that forwarding with a
/// second state machine. That is not free in the one caller that matters — the
/// desktop's history read is the deepest `await` chain in the host
/// (`remote_host::business` → `history::execute` → the page read → the agent
/// call), and it runs on a worker thread with the platform's default stack.
/// Measured on the `remote::commands::bridge_tests` group, which reaches it: the
/// redundant nesting cost **78 KiB** of the 2 MiB a worker gets in a debug build
/// (1970 KiB against 1892 KiB when the wrapper is transparent), which is what
/// took `Rust Windows test / host-remote` over the edge (MSVC debug frames are
/// wider than arm64 ones). Keep this a plain `fn`: there is nothing to await on
/// this side of the call.
///
/// It carries **no first-paint bound** ([`PageBudget::UNBOUNDED`]): completing an
/// exchange can make the page as large as the exchange is, and a reader that is
/// not painting a first screen — the CLI's page read, the terminal's backward
/// walk, and the agent's own paging contract test — asked for the exchange, not
/// for a screenful of it. A caller that *is* painting a first screen bounds it
/// explicitly: the host passes [`PageBudget::FIRST_PAINT`] through
/// [`read_page_with`], the terminal goes through [`read_page_from`].
pub fn read_page<Entry, F, Fut>(
    command: RpcCommand,
    execute: F,
) -> impl Future<Output = Result<SessionEntriesPage<Entry>, String>>
where
    Entry: HistoryEntry,
    F: FnMut(RpcCommand) -> Fut,
    Fut: Future<Output = Result<SessionEntriesPage<Entry>, String>>,
{
    read_page_with(
        command,
        ExchangeStart::Complete,
        PageBudget::UNBOUNDED,
        |entry: &Entry| entry.page_bytes(),
        execute,
    )
}

/// [`read_page`] for a first screen: complete the exchange, bounded by
/// [`PageBudget::FIRST_PAINT`].
///
/// This is the terminal's first page. [`read_page`] is the same walk without the
/// bound, for readers that asked for the exchange itself.
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
/// will send: the host weighs a page the way its lean lane will ship it, and the
/// entries below weigh the row itself. A budget is a first paint's; a caller with
/// no screen to paint passes [`PageBudget::UNBOUNDED`] (which is what
/// [`read_page`] does). A test drives a small budget to pin the truncation.
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
            // Only a page that is asked to begin at a question is bounded: a page
            // allowed to begin anywhere (`ExchangeStart::Anywhere`, the terminal's
            // switch and page-up) is returned as the agent cut it, unbounded and
            // uncompleted — see the two variants on `ExchangeStart`.
            let needs_prefix = start == ExchangeStart::Complete
                && accumulated
                    .first()
                    .is_some_and(HistoryEntry::needs_user_prefix);
            // Walking up is what a page that begins mid-run is worth, but only
            // while the window holds no question at all to be cut to: one more
            // slice is the difference between a page and a fragment of one turn,
            // and a window that already holds a question does not need it (the
            // cut below lands there — cheaper than a round trip, and it keeps
            // the page's own turns whole). The walk is bounded too, so that
            // reaching the question of an enormous turn does not read the whole
            // turn back in for a first paint (that is the 10 MB page this budget
            // retired) — but only for a budget that bounds the page as well
            // (`PageBudget::walk_limit`).
            let has_question = accumulated.iter().any(|entry| !entry.needs_user_prefix());
            let walked: usize = accumulated.iter().map(&weigh).sum();
            if start == ExchangeStart::Complete {
                if needs_prefix && page.has_more && !has_question && walked <= budget.walk_limit() {
                    if accumulated.is_empty()
                        || page.next_offset >= command.before.unwrap_or(i64::MAX)
                    {
                        return Err("history returned a non-advancing backward cursor".into());
                    }
                    command.before = Some(page.next_offset);
                    command.limit = Some(1);
                    continue;
                }
                let trim = trim_to_budget(&mut accumulated, budget, &weigh);
                // What left the page is reachable, not lost: the cursor names the
                // first row this page still carries.
                page.next_offset += trim.dropped as i64;
                if trim.dropped > 0 {
                    page.has_more = true;
                }
                // The page begins mid-run: the rows above it are behind the
                // cursor, but its own first turn is cut, and a fragment projects
                // to nothing. That is the reader's business, so the page states
                // it.
                page.truncated = !trim.begins_exchange;
            }
            page.entries = accumulated;
        }
        return Ok(page);
    }
}

/// What [`trim_to_budget`] had to give up, and whether the page still reads as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Trim {
    /// How many of the oldest rows left the page. The caller's backward cursor
    /// moves up by exactly this, so it names the first row the page carries.
    dropped: usize,
    /// Whether the row the page now starts at is an exchange's first row, so the
    /// page still begins at a question rather than mid-run.
    begins_exchange: bool,
}

/// Drop rows the page cannot afford, without ever leaving it without its question.
///
/// **A page that begins mid-run is the defect this function kept causing.** Two
/// rules, in this order, and the second one wins:
///
/// * A page is a slice of a conversation being read front to back, so it begins
///   at the question its reply answers — the row bound yields to that, because a
///   first page with no question above its reply projects to *no turns at all*
///   (`turnFromAcc` builds a turn from its question). Measured on this machine's
///   store (2026-11-17, 332 first pages read through this function): **7** came
///   back with no user row in them at all — seven conversations that opened
///   blank, and every one of them was a turn of more than 700 rows, where the
///   row cut below had thrown the question away.
/// * An exchange is indivisible. The byte bound is respected by dropping whole
///   *older* exchanges, never by splitting the one the page lands in: cutting
///   inside it either discards the question (the blank conversation again) or
///   opens a hole between the question and the newest rows, and rows in that
///   hole are reachable by no cursor — the page above is fetched from the
///   question, so the middle is gone for good. A turn larger than the byte bound
///   is therefore served whole: one exchange bigger than a reply is what the
///   reply lane's chunking exists for
///   (`prepare_backward_entries_page_with_cap` never splits an exchange either),
///   and a page may be as large as the one turn the reader opened the
///   conversation to read.
///
/// Which is why the cases, in order, are:
///
/// * the whole page fits — nothing is dropped;
/// * some exchange boundary in the page fits both bounds, or (failing that) the
///   byte bound alone — cut there, and the page begins at that exchange's
///   question;
/// * nothing fits and the page already begins at its question — the turn is
///   bigger than a page on its own and leaves whole;
/// * nothing fits and the window begins mid-run — no question came with it, so
///   the newest rows that fit are served and the page says it starts mid-run.
///   Only a walk that hit its own bound gets here; the caller walks first
///   precisely so that this is the rare case and not the blank page.
fn trim_to_budget<Entry, W>(entries: &mut Vec<Entry>, budget: PageBudget, weigh: &W) -> Trim
where
    Entry: HistoryEntry,
    W: Fn(&Entry) -> usize,
{
    // An empty page is an empty conversation, not a cut one: there is nothing to
    // begin at and nothing to say about it.
    if entries.is_empty() {
        return Trim {
            dropped: 0,
            begins_exchange: true,
        };
    }

    let weights: Vec<usize> = entries.iter().map(weigh).collect();
    let mut total: usize = weights.iter().sum();
    let fits = |rows: usize, total: usize| total <= budget.bytes && rows <= budget.rows;

    // A page that fits and opens at a question is done. One that fits but opens
    // mid-run is not: its first rows belong to a turn whose question is above
    // them, so the cut below takes the first question the window holds — those
    // rows stay reachable behind the cursor, and the page opens readable.
    if !entries[0].needs_user_prefix() && fits(entries.len(), total) {
        return Trim {
            dropped: 0,
            begins_exchange: true,
        };
    }

    // Every row that starts an exchange, oldest first: a page cut at one of these
    // begins at a question, which is what the reader is owed. The widest such
    // window is taken — cutting at the newest boundary would throw away
    // exchanges the budget could afford.
    //
    // The widest exchange boundary that fits both bounds.
    let mut remaining = total;
    for start in 0..entries.len() {
        if remaining <= budget.bytes
            && entries.len() - start <= budget.rows
            && !entries[start].needs_user_prefix()
        {
            if start > 0 {
                entries.drain(..start);
            }
            return Trim {
                dropped: start,
                begins_exchange: true,
            };
        }
        remaining -= weights[start];
    }

    // No boundary fits the row bound — the newest turn is longer than it on its
    // own, so every boundary that carries that turn exceeds it. The row bound
    // yields, as little as it can: the *newest* boundary that fits the byte
    // bound is taken, which keeps the page to the one turn the reader opened the
    // conversation for instead of the several that reaching further back would
    // pull in. It also settles a page that begins at its own question: cutting at
    // row zero is a no-op, and the page leaves whole even though it is longer
    // than the row bound.
    let mut carried = 0usize;
    for start in (0..entries.len()).rev() {
        carried += weights[start];
        if carried <= budget.bytes && !entries[start].needs_user_prefix() {
            if start > 0 {
                entries.drain(..start);
            }
            return Trim {
                dropped: start,
                begins_exchange: true,
            };
        }
    }

    // Nothing fits, and the page begins at its own question: the turn is larger
    // than the budget and indivisible (see above). It leaves whole.
    if !entries[0].needs_user_prefix() {
        return Trim {
            dropped: 0,
            begins_exchange: true,
        };
    }

    // Nothing in the window opens an exchange: the walk that completes it hit
    // its bound, so the newest rows that fit are the truthful answer — a page
    // that says it starts mid-run, and can do no better than that.
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
    Trim {
        dropped,
        begins_exchange: !entries[0].needs_user_prefix(),
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

    /// A page with no question in it at all — rows written before exchanges were
    /// marked as such — falls back to the newest rows that fit.
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
    async fn a_page_with_no_question_in_it_falls_back_to_the_newest_rows_that_fit() {
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
        // Nothing above it: the walk has nowhere to go, which is the shape of a
        // session written before exchanges were marked as such.
        let (result, seen) =
            scripted::<Value>(command, vec![page(None, rows.clone(), false, 1_000)]).await;

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

    /// The shared walk is not a first paint: a caller that asked for the exchange
    /// gets the exchange, however much it weighs.
    ///
    /// The budget above belongs to a caller painting a first screen. [`read_page`]
    /// is the same walk for readers with no screen to paint — the CLI's page read
    /// and the agent's own paging contract test
    /// (`a_backward_exchange_larger_than_the_budget_is_complete_before_projection`,
    /// which drives this function) go through it and assert the exchange is
    /// complete. Bounding them would silently drop the head of what they just
    /// asked for, and a `truncated` flag they do not read would be the only sign.
    ///
    /// The fixture is the reported turn's shape: rows above the whole first-paint
    /// budget on their own, split by the Agent into pages that each still need
    /// the older one. Removing the budget from [`read_page`] (back to
    /// [`PageBudget::FIRST_PAINT`]) makes this fail on the second `seen` entry.
    #[tokio::test]
    async fn the_shared_walk_completes_an_exchange_larger_than_a_first_paint() {
        let rows = turn_rows(30, 40_000);
        let whole: usize = rows.iter().map(HistoryEntry::page_bytes).sum();
        assert!(
            whole > FIRST_PAGE_BYTE_BUDGET,
            "the fixture has to be the case this bound is for: {whole} bytes"
        );
        let question = json!({"id": "u0", "kind": "user", "role": "user"});
        let mut completed = vec![question.clone()];
        completed.extend(rows.clone());
        let mut pages = std::collections::VecDeque::from(vec![
            page(None, rows, true, 1_000),
            page(None, vec![question.clone()], false, 0),
        ]);
        let seen = std::cell::RefCell::new(Vec::new());
        let result = {
            let mut execute = |command: RpcCommand| {
                seen.borrow_mut().push(command);
                std::future::ready(Ok(pages.pop_front().expect("scripted page")))
            };
            read_page::<Value, _, _>(
                RpcCommand {
                    before: Some(i64::MAX),
                    ..Default::default()
                },
                &mut execute,
            )
            .await
        };
        let served = result.expect("a completed exchange is a page");
        assert_eq!(
            seen.into_inner().len(),
            2,
            "the walk fetches the rest of the turn instead of stopping at a first-paint budget"
        );
        assert!(!served.truncated, "nothing was left behind");
        assert_eq!(served.entries.len(), completed.len());
        assert_eq!(
            served.entries[0], question,
            "the page begins at the question it answers"
        );
    }

    /// The walk itself has no backstop when the page has no bound.
    ///
    /// [`COMPLETION_WALK_BYTES`] exists so a *first paint* does not read an
    /// enormous turn back in to find its question. A caller with no screen to
    /// paint asked for the exchange, so stopping the walk at that backstop hands
    /// it the turn's tail: the row cut below then has no question to land on and
    /// drops the head of the turn it just asked for. The shape below is the one
    /// the Agent's own paging contract test drives
    /// (`a_backward_exchange_larger_than_the_budget_is_complete_before_projection`):
    /// a single exchange whose rows weigh more than the backstop on their own.
    ///
    /// Weights are the caller's (a page is bounded *as it is sent*), so each row
    /// here is one mebibyte however small it is. Applying the backstop to an
    /// unbounded budget makes this fail on the served page's first row, which
    /// comes back assistant instead of the question.
    #[tokio::test]
    async fn a_page_with_no_bound_walks_all_the_way_to_its_question() {
        fn mib(_: &Value) -> usize {
            1024 * 1024
        }
        let question = json!({"id": "u0", "kind": "user", "role": "user"});
        let answers = turn_rows(11, 8);
        let mut turn = vec![question.clone()];
        turn.extend(answers.clone());
        assert_eq!(turn.len(), 12);
        assert!(
            turn.len() * mib(&Value::Null) > COMPLETION_WALK_BYTES,
            "the fixture has to outrun the first paint's walk backstop"
        );
        // The Agent cuts the turn into slices that each still need the older one;
        // only the last one carries the question.
        let pages = vec![
            page(None, answers[8..].to_vec(), true, 9),
            page(None, answers[4..8].to_vec(), true, 5),
            page(None, answers[..4].to_vec(), true, 1),
            page(None, vec![question.clone()], false, 0),
        ];

        let (result, seen) = scripted_with_budget(
            RpcCommand {
                before: Some(i64::MAX),
                ..Default::default()
            },
            ExchangeStart::Complete,
            PageBudget::UNBOUNDED,
            mib,
            pages,
        )
        .await;

        let served = result.expect("a completed exchange is a page");
        assert_eq!(
            seen.len(),
            4,
            "the walk reads past the first paint's backstop to the question"
        );
        assert!(!served.truncated, "nothing was left behind");
        assert_eq!(
            served.entries, turn,
            "the page is the exchange the caller asked for, whole and in order"
        );
        assert_eq!(
            served.next_offset, 0,
            "the cursor names the row the page still carries"
        );
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

    /// An empty page is an empty conversation, not a cut one.
    ///
    /// The desktop tells a conversation that has not written anything yet from
    /// one whose history is missing by this page; indexing row zero of it would
    /// panic that read out (found by the host's `an_empty_tail_page_belongs_to_
    /// the_live_turn_that_has_not_landed_yet`).
    #[tokio::test]
    async fn an_empty_page_is_not_a_cut_page() {
        let (result, _) = scripted::<Value>(
            RpcCommand {
                before: Some(i64::MAX),
                ..Default::default()
            },
            vec![page(None, Vec::<Value>::new(), false, 0)],
        )
        .await;
        let served = result.expect("a page");
        assert!(served.entries.is_empty());
        assert!(!served.truncated, "there is no turn to have been cut");
        assert_eq!(served.next_offset, 0);
    }

    /// A page allowed to begin anywhere is not bounded either.
    ///
    /// The terminal's switch and page-up read `ExchangeStart::Anywhere` and take
    /// what the agent cut, unbounded: completing or trimming it here would undo
    /// the bound the repeated reads exist to keep. Pinned because the trimming
    /// that a `Complete` page needs would otherwise leak into this one — the
    /// same call, one variant apart.
    #[tokio::test]
    async fn a_page_that_may_begin_anywhere_is_not_trimmed() {
        let mut rows = run_rows("mid", 399);
        rows.push(question("u1"));
        rows.extend(turn_rows(150, 4_000));
        let whole: usize = rows.iter().map(HistoryEntry::page_bytes).sum();
        assert!(
            whole > FIRST_PAGE_BYTE_BUDGET && rows.len() > PageBudget::FIRST_PAINT.rows,
            "the fixture has to be over both bounds: {whole} bytes, {} rows",
            rows.len()
        );

        let (result, seen) = scripted_from::<Value>(
            RpcCommand {
                before: Some(i64::MAX),
                ..Default::default()
            },
            ExchangeStart::Anywhere,
            vec![page(None, rows.clone(), false, 9_000)],
        )
        .await;

        let served = result.expect("a page");
        assert_eq!(seen.len(), 1, "no prefix fetch, no completion");
        assert_eq!(
            served.entries, rows,
            "the page is what the agent cut, not what the budget would keep"
        );
        assert_eq!(served.next_offset, 9_000, "the cursor is the agent's");
        assert!(
            !served.truncated,
            "nothing here claims a cut it did not make"
        );
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

    /// The row bound still bounds a page — where it can do so without losing the
    /// question.
    ///
    /// A thousand rows of a few bytes each are nowhere near the byte budget and
    /// are still a thousand row models to build before the newest screen is on
    /// screen, so a page made of whole exchanges is cut to the newest window the
    /// row bound affords, at a question. What the row bound may **not** do is cut
    /// a page that begins at its own question — that page is one turn, the reader
    /// is reading it, and the turn above the cut is not the one they landed on.
    #[tokio::test]
    async fn the_row_bound_still_cuts_a_page_at_a_question() {
        let mut rows = vec![question("u1")];
        rows.extend(run_rows("e1", 100));
        rows.push(question("u2"));
        rows.extend(run_rows("e2", 249));
        assert_eq!(rows.len(), 351);

        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };
        let (result, _) =
            scripted::<Value>(command, vec![page(None, rows.clone(), true, 5_000)]).await;
        let served = result.expect("page");
        assert!(
            served.entries.len() <= PageBudget::FIRST_PAINT.rows,
            "the row bound is what bounded it: {} rows",
            served.entries.len()
        );
        assert_eq!(
            served.entries[0],
            question("u2"),
            "and the cut lands on the newest question that fits, not mid-run"
        );
        assert_eq!(
            served.next_offset,
            5_000 + 101,
            "the cursor moves up by exactly the rows that left the page"
        );
        assert!(
            !served.truncated,
            "a whole exchange is what the reader was owed"
        );
        let bytes: usize = served.entries.iter().map(HistoryEntry::page_bytes).sum();
        assert!(
            bytes <= FIRST_PAGE_BYTE_BUDGET,
            "and the byte bound still holds for what the reply carries: {bytes} bytes"
        );
    }

    /// The budget is spent on what will be sent, not on what was read.
    ///
    /// The host weighs a lean page by what its trim leaves; the same bytes then
    /// buy several times the rows a raw weight buys. This is the property the
    /// two weights differ for — if the caller's weight were ignored, the two
    /// pages would be identical.
    ///
    /// Both pages keep a question: the raw weight can only afford the newest
    /// exchange and cuts there, the lean weight affords all three and is served
    /// whole. Three exchanges, because asking a page to shed is what a byte bound
    /// is for — one exchange is indivisible and leaves whole either way.
    #[tokio::test]
    async fn a_cheaper_weight_fits_more_rows_in_the_same_budget() {
        let mut rows = vec![question("u1")];
        rows.extend(turn_rows(80, 4_000));
        rows.push(question("u2"));
        rows.extend(turn_rows(80, 4_000));
        rows.push(question("u3"));
        rows.extend(turn_rows(80, 4_000));
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
        let raw_bytes: usize = raw.entries.iter().map(HistoryEntry::page_bytes).sum();
        assert!(
            raw_bytes <= FIRST_PAGE_BYTE_BUDGET,
            "the raw page fits what will be sent: {raw_bytes} bytes"
        );
        // The lean page is bounded by the lean weight — what it will actually
        // cost on the wire — rather than by its raw bodies.
        let lean_bytes: usize = lean.entries.iter().map(|_| 80).sum();
        assert!(lean_bytes <= FIRST_PAGE_BYTE_BUDGET, "{lean_bytes} bytes");
        assert_eq!(
            lean.entries[0],
            question("u1"),
            "and begins at the question"
        );
        assert_eq!(
            raw.entries[0],
            question("u3"),
            "the raw page begins at the question of the exchange it could afford"
        );
    }

    /// A conversation numbering its rows the way the store does: row *i* has
    /// ordinal *i*, so a page's first row is also the cursor that names it.
    ///
    /// Questions at 0, 100, 200 and 300, and a newest run of **400** rows —
    /// longer than the first paint's row bound, which is the blank page's shape.
    fn conversation() -> Vec<Value> {
        let mut rows = vec![question("q0")];
        rows.extend(run_rows("e0", 99));
        rows.push(question("q1"));
        rows.extend(run_rows("e1", 99));
        rows.push(question("q2"));
        rows.extend(run_rows("e2", 99));
        rows.push(question("q3"));
        rows.extend(run_rows("e3", 399));
        assert_eq!(rows.len(), 700);
        rows
    }

    fn ids(rows: &[Value]) -> Vec<String> {
        rows.iter()
            .map(|row| row["id"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// The page-up and the first paint are two slices of one conversation: the
    /// cursor of the first names the row above which the second continues.
    ///
    /// This is also the three-path property: the desktop's streaming and
    /// non-streaming opens share this one read (the live bubble is an overlay on
    /// top of it, never a different history window), so what is asserted here is
    /// what both paths see — the same cursor, the same rows.
    ///
    /// The Agent cuts a backward page at 150 rows, so the first paint here is two
    /// fetches: the newest 150 rows, then the slice that completes the turn to
    /// its question. Concatenated with the page-up they are exactly
    /// `conversation()[100..]` — no gap, no repeat, and the cursor at its head.
    #[tokio::test]
    async fn the_first_paint_and_the_page_above_it_are_one_contiguous_suffix() {
        let history = conversation();
        let tail = |range: std::ops::RangeInclusive<usize>| history[range].to_vec();

        let (first, seen) = scripted::<Value>(
            RpcCommand {
                before: Some(i64::MAX),
                limit: Some(10),
                ..Default::default()
            },
            vec![
                page(None, tail(550..=699), true, 550),
                page(None, tail(300..=549), true, 300),
            ],
        )
        .await;
        let first = first.expect("first paint");
        assert_eq!(seen.len(), 2, "the turn was completed to its question");
        assert_eq!(ids(&first.entries), ids(&history[300..=699]));
        assert_eq!(
            first.next_offset, 300,
            "the cursor is the ordinal of the first row the page carries"
        );
        assert!(!first.truncated, "the whole turn was served");

        let (older, _) = scripted::<Value>(
            RpcCommand {
                before: Some(first.next_offset),
                limit: Some(10),
                ..Default::default()
            },
            vec![page(None, tail(100..=299), true, 100)],
        )
        .await;
        let older = older.expect("page up");
        assert_eq!(ids(&older.entries), ids(&history[100..=299]));

        // No repeat, no gap: the two pages are the suffix they claim to be.
        let mut joined = ids(&older.entries);
        joined.extend(ids(&first.entries));
        assert_eq!(joined, ids(&history[100..=699]));
        assert_eq!(
            older.next_offset, 100,
            "and its own cursor is the row it starts at"
        );
    }

    /// A page-up lands mid-run like a first paint does, and may not come back
    /// without the question it landed on either.
    ///
    /// Scrolling up asks for a slice of the *same* oversized turn; the walk
    /// completes it back to its question and the row bound then yields to it.
    /// Reverting the walk serves 399 assistant rows here instead of the turn's
    /// 399 rows from `q3`.
    #[tokio::test]
    async fn a_page_up_keeps_the_question_it_lands_in() {
        let history = conversation();
        let (result, seen) = scripted::<Value>(
            RpcCommand {
                before: Some(699),
                limit: Some(10),
                ..Default::default()
            },
            vec![
                page(None, history[550..=698].to_vec(), true, 550),
                page(None, history[300..=549].to_vec(), true, 300),
            ],
        )
        .await;

        let served = result.expect("page up");
        assert_eq!(seen.len(), 2, "one slice, then the question");
        assert_eq!(
            served.entries.first(),
            Some(&history[300]),
            "the page begins at the question it landed in, not mid-run"
        );
        assert_eq!(served.entries.len(), 399);
        assert_eq!(served.next_offset, 300);
    }

    /// The same cursor serves the same rows, twice: a history read has no state
    /// that a second reader could see differently.
    #[tokio::test]
    async fn the_same_cursor_serves_the_same_rows_every_time() {
        let history = conversation();
        let pages = || {
            vec![
                page(None, history[550..=699].to_vec(), true, 550),
                page(None, history[300..=549].to_vec(), true, 300),
            ]
        };
        let read = |pages: Vec<SessionEntriesPage<Value>>| async move {
            scripted::<Value>(
                RpcCommand {
                    before: Some(i64::MAX),
                    limit: Some(10),
                    ..Default::default()
                },
                pages,
            )
            .await
            .0
            .expect("page")
        };
        let live = read(pages()).await;
        let settled = read(pages()).await;
        assert_eq!(ids(&live.entries), ids(&settled.entries));
        assert_eq!(live.next_offset, settled.next_offset);
        assert_eq!(live.truncated, settled.truncated);
    }

    /// A mid-run window is not something to cut: it is a page that has not
    /// reached its question yet, so the walk continues until it has.
    ///
    /// This is the reported blank conversation in the shape the store produced it
    /// (2026-11-17): the Agent cuts a backward read at its own row ceiling, so the
    /// first two slices here (150 and 600 rows of one long run) hold no question
    /// at all. The row bound used to cut that window to its newest 300 rows —
    /// every one of them an assistant or tool row — and the page projected to
    /// zero turns. Seven conversations on this machine were served exactly that.
    /// Reverting the walk makes the first assertion below see an assistant row
    /// instead of `q0`.
    #[tokio::test]
    async fn a_mid_run_window_keeps_walking_until_it_reaches_its_question() {
        let mut history = vec![question("q0")];
        history.extend(run_rows("e0", 900));
        assert_eq!(history.len(), 901);

        let (result, seen) = scripted::<Value>(
            RpcCommand {
                before: Some(i64::MAX),
                limit: Some(10),
                ..Default::default()
            },
            vec![
                page(None, history[751..=900].to_vec(), true, 751),
                page(None, history[151..=750].to_vec(), true, 151),
                page(None, history[0..=150].to_vec(), false, 0),
            ],
        )
        .await;

        let served = result.expect("page");
        assert_eq!(
            served.entries.first(),
            Some(&history[0]),
            "the page reaches and keeps the question it was walking to"
        );
        assert_eq!(
            seen.len(),
            3,
            "two mid-run slices were not enough, and the walk said so instead of cutting"
        );
        assert!(!served.truncated, "the turn is served from its question");
        assert_eq!(served.entries.len(), history.len());
        assert_eq!(
            served.next_offset, 0,
            "the cursor reached the top of history"
        );
    }

    /// A turn too heavy to reach within the budget is walked to anyway, and then
    /// served whole.
    ///
    /// This is the other half of the blank report: the window exceeds the byte
    /// budget before a question is in it, and the old cut kept the newest rows —
    /// the reply with nothing above it. The walk spends one slice past the budget
    /// on reaching the question, because a page is worthless without it.
    ///
    /// Once the question is here the page is one exchange, and an exchange is
    /// indivisible: the question without its reply, or the reply without its
    /// question, is not a page, and a hole between them is reachable by no
    /// cursor. So it leaves whole — one turn bigger than a reply is the case the
    /// reply lane's chunking carries (`a_page_larger_than_one_reply_arrives_
    /// intact` in the host's tests).
    #[tokio::test]
    async fn a_turn_over_the_byte_budget_is_walked_to_its_question_and_served_whole() {
        let mut history = vec![question("q0")];
        history.extend(turn_rows(400, 4_000));
        let whole: usize = history.iter().map(HistoryEntry::page_bytes).sum();
        assert!(whole > FIRST_PAGE_BYTE_BUDGET, "{whole} bytes");

        let (result, seen) = scripted::<Value>(
            RpcCommand {
                before: Some(i64::MAX),
                limit: Some(10),
                ..Default::default()
            },
            vec![
                page(None, history[250..=400].to_vec(), true, 250),
                page(None, history[0..=249].to_vec(), false, 0),
            ],
        )
        .await;

        let served = result.expect("page");
        assert_eq!(
            served.entries.first(),
            Some(&history[0]),
            "the page keeps the question it walked to"
        );
        assert_eq!(
            served.entries.len(),
            history.len(),
            "and the whole turn with it: the question without its reply, or the \
             reply without its question, is not a page"
        );
        assert_eq!(
            served.entries.last(),
            Some(&history[400]),
            "the newest row is what a reader opened the conversation for"
        );
        assert!(
            !served.truncated,
            "nothing was left out, so nothing is claimed missing"
        );
        assert!(
            whole > FIRST_PAGE_BYTE_BUDGET,
            "and this is the case a page is allowed to be that large in: one turn"
        );
        assert_eq!(
            seen.len(),
            2,
            "one slice past the budget, and no more: the question was in the second"
        );
    }

    /// A row that opens an exchange: the reader is owed the page from here up.
    fn question(id: &str) -> Value {
        json!({"id": id, "kind": "user", "role": "user"})
    }

    /// One exchange's worth of a busy run: `rows` assistant steps above `id`.
    fn run_rows(id: &str, rows: usize) -> Vec<Value> {
        (0..rows)
            .map(|index| {
                json!({
                    "id": format!("{id}-a{index}"),
                    "kind": "assistant",
                    "role": "assistant",
                    "blocks": [{"kind": "text", "text": "x"}],
                })
            })
            .collect()
    }

    /// The cut that ends a too-large page lands on an exchange boundary, so the
    /// page still begins at a question.
    ///
    /// This is the reported defect (2026-11-16). A first page is completed back
    /// to its question; when the window the walk accumulates busts the budget the
    /// page is cut, and the cut used to fall wherever the byte count ran out —
    /// which is mid-run, because the Agent cuts a page-up mid-exchange too.
    /// Measured on that machine's store: **107** of 1,158 conversations opened on
    /// a page whose first row was an assistant step, i.e. with no question above
    /// the reply the reader landed on; three of them were the reported
    /// conversations.
    ///
    /// The window here is that shape: its oldest row is the tail of an exchange
    /// the Agent cut, and the questions below it are what the cut can land on.
    /// Reverting the boundary walk (cutting row by row again) makes this fail on
    /// the `role` assertion, which is the property the reader sees.
    #[tokio::test]
    async fn a_page_cut_to_fit_still_begins_at_a_question() {
        // Oldest first: the tail of a cut exchange, then two whole exchanges.
        let mut older = vec![json!({"id": "cut", "kind": "assistant", "role": "assistant"})];
        older.push(question("u1"));
        older.extend(run_rows("e1", 250));
        older.push(question("u2"));
        older.extend(run_rows("e2", 100));
        let newest = run_rows("tail", 60);

        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };
        let (result, seen) = scripted::<Value>(
            command,
            vec![
                page(None, newest.clone(), true, 8_000),
                page(None, older, true, 4_000),
            ],
        )
        .await;

        let served = result.expect("a cut page is a page");
        assert_eq!(seen.len(), 2, "the question was fetched before cutting");
        assert!(
            served.entries.len() <= PageBudget::FIRST_PAINT.rows,
            "the budget still bounds the page: {} rows",
            served.entries.len()
        );
        assert_eq!(
            served.entries[0]["role"], "user",
            "the cut lands on the exchange it keeps, not mid-run"
        );
        assert!(
            !served.truncated,
            "a page that begins at a question is not a turn's tail"
        );
        // The cursor names the oldest row the page still carries, so what left
        // the page is reachable above it rather than lost.
        let dropped = 413 - served.entries.len();
        assert_eq!(
            served.next_offset,
            4_000 + dropped as i64,
            "the cursor moves up by exactly the rows that left the page"
        );
        assert!(served.has_more, "older history is still reachable");
    }

    /// When a turn is bigger than the *byte* budget, it is served whole — from
    /// its question, not as the fragment that opened blank.
    ///
    /// This used to be the "starts mid-run" case, and it was wrong: the row cut
    /// kept the newest rows that fit and threw the question away, so the page
    /// held a reply with nothing above it. Measured on this machine's store
    /// (2026-11-17, 332 first pages): seven conversations were served exactly
    /// that page and opened blank, because `turnFromAcc` returns null for a page
    /// whose first row is not a question.
    ///
    /// The page cannot be made both readable and small: one exchange is
    /// indivisible, and this fixture is a single one. Keeping the question and
    /// dropping the newest rows would hide the reply the reader opened the
    /// conversation for; keeping a middle hole would put rows out of every
    /// cursor's reach. So it leaves whole and the reply lane chunks it.
    ///
    /// Reverting to the row cut makes the first assertion below fail: the page
    /// would begin with an assistant row.
    #[tokio::test]
    async fn a_turn_too_big_for_the_byte_budget_is_served_whole_from_its_question() {
        let mut rows = vec![question("u1")];
        rows.extend(turn_rows(400, 4_000));
        let whole: usize = rows.iter().map(HistoryEntry::page_bytes).sum();
        assert!(
            whole > FIRST_PAGE_BYTE_BUDGET,
            "the fixture has to be the case this bound is for: {whole} bytes"
        );

        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };
        let (result, _) =
            scripted::<Value>(command, vec![page(None, rows.clone(), true, 4_000)]).await;

        let served = result.expect("a page is a page");
        assert_eq!(
            served.entries[0], rows[0],
            "the page keeps the question it landed on"
        );
        assert_eq!(
            served.entries.len(),
            rows.len(),
            "and carries the turn it opened: {} of {} rows",
            served.entries.len(),
            rows.len()
        );
        assert_eq!(
            served.entries.last().map(HistoryEntry::entry_id),
            rows.last().map(HistoryEntry::entry_id),
            "the newest row is still the newest row"
        );
        assert!(
            !served.truncated,
            "nothing is missing, so the page does not claim to start mid-run"
        );
        assert_eq!(
            served.next_offset, 4_000,
            "the cursor names the first row the page carries — the question"
        );
        assert!(
            served.has_more,
            "the rows above it are still reachable behind the cursor"
        );
    }

    /// A turn longer than the row bound is served whole, from its question.
    ///
    /// This is the reported blank conversation (2026-11-17): a 400-row turn
    /// whose bytes are a fraction of the budget, stopped by the 300-row render
    /// bound alone. The bound is real, but a page that stays inside it can only
    /// begin mid-run here — and then it projects to **no turns**, because
    /// `turnFromAcc` builds a turn from its question. The question wins: the row
    /// bound yields, the byte bound does not.
    ///
    /// Reverting the fix makes `entries[0]` an assistant row and `truncated`
    /// true, which is the blank page.
    #[tokio::test]
    async fn a_turn_longer_than_the_row_bound_is_served_from_its_question() {
        let mut rows = vec![question("u1")];
        rows.extend(run_rows("e1", 399));
        assert_eq!(rows.len(), 400);

        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };
        let (result, _) =
            scripted::<Value>(command, vec![page(None, rows.clone(), true, 1_000)]).await;

        let served = result.expect("page");
        assert_eq!(
            served.entries.len(),
            rows.len(),
            "the turn is served whole rather than cut to the row bound"
        );
        assert_eq!(
            served.entries[0], rows[0],
            "and it begins at that turn's question, which is what the projection needs"
        );
        assert!(
            !served.truncated,
            "nothing was left out, so nothing is claimed to be missing"
        );
        assert_eq!(served.next_offset, 1_000);
        let bytes: usize = served.entries.iter().map(HistoryEntry::page_bytes).sum();
        assert!(bytes <= FIRST_PAGE_BYTE_BUDGET, "{bytes} bytes");
    }

    /// Whatever the row budget, the page begins at a question when one exists,
    /// and never costs more than the byte budget.
    ///
    /// The three budgets are the interesting ones: smaller than a single row, the
    /// shipped first paint, and no bound at all. The rule they all obey is the
    /// one the reader sees — the question is always in the page — and the one
    /// that must survive it: the page still fits a reply.
    #[tokio::test]
    async fn a_first_page_begins_at_a_question_at_any_row_budget() {
        let mut rows = vec![question("u1")];
        rows.extend(run_rows("e1", 399));
        for budget_rows in [1_usize, PageBudget::FIRST_PAINT.rows, 10_000] {
            let command = RpcCommand {
                before: Some(i64::MAX),
                ..Default::default()
            };
            let (result, _) = scripted_with_budget(
                command,
                ExchangeStart::Complete,
                PageBudget {
                    bytes: FIRST_PAGE_BYTE_BUDGET,
                    rows: budget_rows,
                },
                |entry: &Value| entry.page_bytes(),
                vec![page(None, rows.clone(), true, 1_000)],
            )
            .await;

            let served = result.expect("page");
            assert_eq!(
                served.entries[0], rows[0],
                "rows={budget_rows}: the page begins at the question"
            );
            let bytes: usize = served.entries.iter().map(HistoryEntry::page_bytes).sum();
            assert!(
                bytes <= FIRST_PAGE_BYTE_BUDGET,
                "rows={budget_rows}: the page still fits a reply: {bytes} bytes"
            );
        }
    }

    /// The boundary walk keeps the *widest* window that fits.
    ///
    /// Cutting at the oldest boundary that fits is what makes the page as full as
    /// the budget allows; cutting at the newest would throw away exchanges the
    /// budget could afford, which is the same defect from the other side.
    #[tokio::test]
    async fn the_boundary_cut_keeps_the_widest_window_that_fits() {
        let mut older = vec![json!({"id": "cut", "kind": "assistant", "role": "assistant"})];
        for exchange in 1..=3 {
            older.push(question(&format!("u{exchange}")));
            older.extend(run_rows(&format!("e{exchange}"), 100));
        }
        let newest = run_rows("tail", 30);

        let command = RpcCommand {
            before: Some(i64::MAX),
            ..Default::default()
        };
        let (result, _) = scripted::<Value>(
            command,
            vec![
                page(None, newest, true, 8_000),
                page(None, older, true, 4_000),
            ],
        )
        .await;

        let served = result.expect("page");
        assert!(
            served.entries.len() <= PageBudget::FIRST_PAINT.rows,
            "the budget bounds it: {} rows",
            served.entries.len()
        );
        assert_eq!(served.entries[0]["role"], "user");
        // Three exchanges of 101 rows do not fit in 300; two do (202). A
        // newest-only cut would have kept just one.
        assert!(
            served.entries.len() > 101,
            "more than the newest exchange alone: {} rows",
            served.entries.len()
        );
    }
}
