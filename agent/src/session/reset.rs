//! In-place history reset — the user-invoked, destructive counterpart of
//! compaction.
//!
//! Why this exists (and why compaction cannot do it): compaction is a *bounded
//! projection* with a hard invariant — `compaction::protected_text` keeps every
//! user original, and `compaction/budget.rs` fails the plan outright rather than
//! dropping one. So the thing that actually kills a long session (user originals
//! accumulated over hundreds of turns, against `MAX_EXPANDED_HISTORY = 128_000`
//! in `compaction/budget.rs`) is exactly the thing compaction is forbidden to
//! touch: once the user originals plus the retained tail no longer fit that
//! ceiling, the session reports "context compaction budget exceeded" and does
//! **not** self-heal. This module is the tool that lifts that ceiling, by
//! deleting the parts of the history the user explicitly chose.
//!
//! The two paths therefore do not share an invariant, deliberately:
//!
//! * compaction (automatic, or one command) may never drop a protected user
//!   original — that is the safety design of an unattended path;
//! * this reset (explicit, backed up, dry-run first) may drop anything the
//!   caller names, including user originals.
//!
//! Do not "unify" them: teaching compaction to drop user originals removes the
//! guarantee that an unattended compaction never loses the user's words, and
//! making this respect the compaction invariant removes the only escape from the
//! 128K ceiling. Everything here is a pure function of the entry list plus the
//! selectors, so the preview the user approves and the rewrite that is executed
//! are produced by the same code.

use super::entry::{
    SessionEntry, ENTRY_TYPE_ASSISTANT, ENTRY_TYPE_COMPACTION, ENTRY_TYPE_LABEL,
    ENTRY_TYPE_MODEL_CHANGE, ENTRY_TYPE_SESSION_INFO, ENTRY_TYPE_THINKING_LEVEL_CHANGE,
    ENTRY_TYPE_TOOL, ENTRY_TYPE_USER,
};
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};

/// Kinds a reset can remove from the part it keeps. A list, not a bitmask, and
/// an unknown name is an error: the user asked for "or whatever else", and a
/// typo that silently kept the data it was meant to drop would be the worst
/// possible failure mode here.
pub const DROP_KIND_NAMES: [&str; 5] = [
    "thinking",
    "tool-calls",
    "tool-results",
    "attachments",
    "compaction",
];

/// Text block the agent injects for attachments that are not images (see
/// `rpc::prompt_helpers::build_user_message_with_model_context`). Recognised so
/// `--drop attachments` removes the manifest together with the attachments it
/// describes, instead of leaving a dangling JSON blob in the prompt.
pub(crate) const ATTACHMENT_MANIFEST_PREFIX: &str =
    "\n\nUser attachment metadata follows as a JSON array.";

/// What a user entry keeps when dropping its attachments would leave it with no
/// content at all. An empty user message is rejected by the providers, and this
/// mirrors the existing `repair::TOOL_LOST_PLACEHOLDER_PREFIX` convention: say
/// plainly what was removed instead of sending an invalid request.
pub(crate) const ATTACHMENT_REMOVED_PLACEHOLDER: &str =
    "[Attachment removed by session history reset]";

/// Error token for "the history moved between planning and applying". Shared so
/// the RPC layer can map it to a retryable code without matching on prose.
pub const HISTORY_CHANGED: &str = "reset_history_changed";

const PREVIEW_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DropKind {
    Thinking,
    ToolCalls,
    ToolResults,
    Attachments,
    Compaction,
}

impl DropKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Thinking => "thinking",
            Self::ToolCalls => "tool-calls",
            Self::ToolResults => "tool-results",
            Self::Attachments => "attachments",
            Self::Compaction => "compaction",
        }
    }

    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "thinking" => Self::Thinking,
            "tool-calls" => Self::ToolCalls,
            "tool-results" => Self::ToolResults,
            "attachments" => Self::Attachments,
            "compaction" => Self::Compaction,
            other => bail!(
                "unknown drop kind `{other}`; known kinds: {}",
                DROP_KIND_NAMES.join(", ")
            ),
        })
    }
}

/// What a reset keeps, and what it removes from the part it keeps.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResetSelectors {
    /// Drop every turn, keeping only the session record.
    pub all: bool,
    /// Keep the first N user turns (None = no bound on that side).
    pub keep_first: Option<u64>,
    /// Keep the last N user turns (composes with `keep_first`: head + tail).
    pub keep_last: Option<u64>,
    /// Kinds to remove from the retained part.
    pub drop: Vec<DropKind>,
}

impl ResetSelectors {
    /// True when the caller selected nothing at all.
    pub fn selects_nothing(&self) -> bool {
        !self.all && self.keep_first.is_none() && self.keep_last.is_none() && self.drop.is_empty()
    }

    /// True when the selection rewrites the turn structure — and therefore
    /// invalidates every compaction checkpoint, which is a projection over the
    /// turns it covers.
    pub fn restricts_turns(&self) -> bool {
        self.all || self.keep_first.is_some() || self.keep_last.is_some()
    }

    pub fn validate(&self) -> Result<()> {
        if self.selects_nothing() {
            bail!("nothing selected: pass --all, --keep-first N, --keep-last N or --drop <kinds>");
        }
        if self.all && (self.keep_first.is_some() || self.keep_last.is_some()) {
            bail!("--all cannot be combined with --keep-first or --keep-last");
        }
        Ok(())
    }

    /// Canonical selector JSON for the response, so a caller can see exactly
    /// which kinds its request resolved to.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "all": self.all,
            "keepFirst": self.keep_first,
            "keepLast": self.keep_last,
            "drop": self.drop.iter().map(|kind| kind.name()).collect::<Vec<_>>(),
        })
    }
}

/// Per-kind effect of the drop pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropEffect {
    pub kind: String,
    /// Entries removed or rewritten by this kind.
    pub entries: u64,
    /// Content blocks removed by this kind.
    pub blocks: u64,
}

/// Head/tail of the retained part, so a preview shows *what* survives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryPreview {
    pub id: String,
    pub kind: String,
    pub role: String,
    pub text: String,
}

/// Everything a caller needs to judge a reset before applying it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPlan {
    pub entries_before: u64,
    pub entries_after: u64,
    pub turns_before: u64,
    pub turns_kept: u64,
    /// Estimated history size, with the estimator compaction budgets with. An
    /// estimate, not the live `contextTokens` (which only moves after the
    /// session's next request).
    pub estimated_tokens_before: u64,
    pub estimated_tokens_after: u64,
    pub drop_effects: Vec<DropEffect>,
    pub first_kept: Option<EntryPreview>,
    pub last_kept: Option<EntryPreview>,
    /// Anything about the result that the numbers alone do not explain.
    pub notes: Vec<String>,
}

/// The plan plus the exact entry list it describes. Kept together so a preview
/// and the rewrite it authorises can never disagree.
#[derive(Debug)]
pub struct ResetPlanResult {
    pub plan: ResetPlan,
    pub retained: Vec<SessionEntry>,
    /// Runs whose streamed journal has to go with the entries: any run that lost
    /// or changed an entry, plus runs that disappeared entirely.
    pub affected_runs: BTreeSet<String>,
}
/// Whether an entry is session *record* rather than conversation history, and is
/// therefore never dropped by a reset: id, name, cwd, model, thinking level,
/// parent and the persistent prompt segments all live in `session_info`, and a
/// `model_change` records a switch the rest of the session depends on.
pub fn is_session_record(entry: &SessionEntry) -> bool {
    matches!(
        entry.entry_type.as_str(),
        ENTRY_TYPE_SESSION_INFO
            | ENTRY_TYPE_MODEL_CHANGE
            | ENTRY_TYPE_THINKING_LEVEL_CHANGE
            | ENTRY_TYPE_LABEL
    )
}

fn entry_run_id(entry: &SessionEntry) -> Option<&str> {
    entry
        .meta
        .as_ref()
        .and_then(|meta| meta.get("run_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
}

/// The call an entry answers. A stored `tool` entry keeps its answer in the
/// `tool_result` block: the envelope's own `tool_call_id` field is only
/// re-derived at load time (`projection::hydrate_entry_projections`), and raw
/// storage reads leave it empty.
fn entry_answers_call(entry: &SessionEntry) -> Option<String> {
    if !entry.tool_call_id.is_empty() {
        return Some(entry.tool_call_id.clone());
    }
    content_blocks(entry).iter().find_map(|block| {
        (block_kind(block) == "tool_result")
            .then(|| {
                block
                    .get("tool_call_id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned)
            })
            .flatten()
    })
}

/// Index of the first entry of every user turn, in order.
fn turn_starts(entries: &[SessionEntry]) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.entry_type == ENTRY_TYPE_USER)
        .map(|(index, _)| index)
        .collect()
}

/// The turn an entry belongs to, or `None` for the preamble before the first
/// user entry. A turn runs from its user entry up to the entry before the next
/// user entry — the same boundary `session forks` uses.
fn turn_of(index: usize, starts: &[usize]) -> Option<usize> {
    starts.iter().rposition(|start| *start <= index)
}

fn kept_turns(selectors: &ResetSelectors, turns: usize) -> Vec<bool> {
    // No turn selector at all: every turn is kept (the drop pass still runs).
    if !selectors.restricts_turns() {
        return vec![true; turns];
    }
    if selectors.all {
        return vec![false; turns];
    }
    let mut kept = vec![false; turns];
    if let Some(first) = selectors.keep_first {
        for flag in kept.iter_mut().take(first as usize) {
            *flag = true;
        }
    }
    if let Some(last) = selectors.keep_last {
        for flag in kept.iter_mut().skip(turns.saturating_sub(last as usize)) {
            *flag = true;
        }
    }
    kept
}

/// Whether anything a provider will accept is left of an entry. An assistant
/// that held only the content a selector just removed is not a valid message
/// (`repair::strip_empty_assistants` would drop it at load anyway); removing it
/// here keeps the stored history and the projection in step.
fn entry_has_a_body(entry: &SessionEntry) -> bool {
    if entry.entry_type != ENTRY_TYPE_ASSISTANT {
        return true;
    }
    entry.content.is_some() || !entry.tool_calls.is_empty()
}

fn block_kind(block: &serde_json::Value) -> &str {
    block
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

fn content_blocks(entry: &SessionEntry) -> Vec<serde_json::Value> {
    match entry.content.as_ref() {
        Some(serde_json::Value::Array(blocks)) => blocks.clone(),
        Some(serde_json::Value::String(text)) => {
            vec![serde_json::json!({"type": "text", "text": text})]
        }
        _ => Vec::new(),
    }
}

fn set_content(entry: &mut SessionEntry, blocks: Vec<serde_json::Value>) {
    entry.content = if blocks.is_empty() {
        None
    } else {
        Some(serde_json::Value::Array(blocks))
    };
}

/// First display-ish text of an entry, for the head/tail preview.
fn preview_of(entry: &SessionEntry) -> EntryPreview {
    let mut text = String::new();
    for block in content_blocks(entry) {
        match block_kind(&block) {
            "text" => text.push_str(
                block
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            ),
            "tool_call" => text.push_str(&format!(
                "{} {}",
                block
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tool"),
                block
                    .get("args")
                    .map(|args| args.to_string())
                    .unwrap_or_default()
            )),
            "tool_result" => text.push_str(
                block
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            ),
            "reasoning" => text.push_str("[reasoning]"),
            "image_url" => text.push_str("[image]"),
            _ => {}
        }
        if !text.is_empty() {
            break;
        }
    }
    if text.trim().is_empty() {
        text = format!("({} entry)", entry.entry_type);
    }
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview: String = cleaned.chars().take(PREVIEW_CHARS).collect();
    if cleaned.chars().count() > PREVIEW_CHARS {
        preview.push('…');
    }
    EntryPreview {
        id: entry.id.clone(),
        kind: entry.entry_type.clone(),
        role: entry.role.clone(),
        text: preview,
    }
}

/// The estimator compaction budgets with, so the preview's token numbers are
/// comparable with the context figure `session status` reports.
fn estimate_entries_tokens(entries: &[SessionEntry]) -> u64 {
    super::build_context(entries)
        .iter()
        .map(|message| crate::compaction::estimate_tokens(message).max(0) as u64)
        .sum()
}

fn record_effect(effects: &mut Vec<DropEffect>, kind: DropKind, entries: u64, blocks: u64) {
    if entries == 0 && blocks == 0 {
        return;
    }
    match effects.iter_mut().find(|effect| effect.kind == kind.name()) {
        Some(effect) => {
            effect.entries += entries;
            effect.blocks += blocks;
        }
        None => effects.push(DropEffect {
            kind: kind.name().to_string(),
            entries,
            blocks,
        }),
    }
}

/// One `--drop` kind applied to one entry. Returns how many content blocks went,
/// and whether the entry survives at all (an assistant that held nothing but the
/// reasoning it just lost is not a valid message).
fn drop_kind_from_entry(entry: &mut SessionEntry, kind: DropKind) -> (u64, bool) {
    // The block-based kinds only apply to conversation messages. A session
    // record's `content` is the metadata object itself (name, cwd, model, prompt
    // segments, cost split), so rebuilding it as a block array would destroy the
    // session's identity — the one thing a reset promises not to touch.
    if matches!(
        kind,
        DropKind::Thinking | DropKind::ToolCalls | DropKind::ToolResults | DropKind::Attachments
    ) && !matches!(
        entry.entry_type.as_str(),
        ENTRY_TYPE_USER | ENTRY_TYPE_ASSISTANT | ENTRY_TYPE_TOOL
    ) {
        return (0, true);
    }
    match kind {
        DropKind::Thinking => {
            let mut blocks = content_blocks(entry);
            let before = blocks.len();
            blocks.retain(|block| block_kind(block) != "reasoning");
            let mut removed = (before - blocks.len()) as u64;
            set_content(entry, blocks);
            // Clear the legacy field too: `entry_records::normalize_entry`
            // re-synthesises a reasoning block from `thinking` on insert, so
            // dropping only the block would let the next write resurrect it.
            if !entry.thinking.is_empty() {
                entry.thinking.clear();
                removed = removed.max(1);
            }
            (removed, entry_has_a_body(entry))
        }
        DropKind::ToolCalls => {
            let mut blocks = content_blocks(entry);
            let before = blocks.len();
            blocks.retain(|block| block_kind(block) != "tool_call");
            let mut removed = (before - blocks.len()) as u64;
            set_content(entry, blocks);
            if !entry.tool_calls.is_empty() {
                entry.tool_calls.clear();
                removed = removed.max(1);
            }
            (removed, entry_has_a_body(entry))
        }
        DropKind::ToolResults => {
            if entry.entry_type != ENTRY_TYPE_TOOL {
                return (0, true);
            }
            // The whole entry goes: a `tool` message exists only to answer a
            // call, and there is no body left to keep once its result is gone.
            let removed = content_blocks(entry).len() as u64;
            (removed, false)
        }
        DropKind::Attachments => {
            let mut blocks = content_blocks(entry);
            let before = blocks.len();
            blocks.retain(|block| {
                if block_kind(block) == "image_url" {
                    return false;
                }
                !(block_kind(block) == "text"
                    && block
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|text| text.starts_with(ATTACHMENT_MANIFEST_PREFIX)))
            });
            let mut removed = (before - blocks.len()) as u64;
            if let Some(meta) = entry.meta.as_mut().and_then(|meta| meta.as_object_mut()) {
                if meta.remove("attachments").is_some() {
                    removed = removed.max(1);
                }
            }
            if entry
                .meta
                .as_ref()
                .is_some_and(|meta| meta.as_object().is_some_and(|object| object.is_empty()))
            {
                entry.meta = None;
            }
            if blocks.is_empty() && entry.tool_calls.is_empty() {
                // Nothing is left to send: keep the message shape valid rather
                // than emitting an empty user turn.
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": ATTACHMENT_REMOVED_PLACEHOLDER,
                }));
            }
            set_content(entry, blocks);
            (removed, true)
        }
        DropKind::Compaction => {
            if entry.entry_type == ENTRY_TYPE_COMPACTION {
                (0, false)
            } else {
                (0, true)
            }
        }
    }
}

/// Compute the reset a set of selectors describes. Pure: the same call produces
/// the preview a user approves and the entry list that gets written.
pub fn plan_reset(entries: &[SessionEntry], selectors: &ResetSelectors) -> Result<ResetPlanResult> {
    selectors.validate()?;
    let before_tokens = estimate_entries_tokens(entries);
    let starts = turn_starts(entries);
    let turns = starts.len();
    let kept = kept_turns(selectors, turns);

    let mut selected: Vec<SessionEntry> = Vec::with_capacity(entries.len());
    let mut affected_runs: BTreeSet<String> = BTreeSet::new();
    let before_runs: BTreeSet<String> = entries
        .iter()
        .filter_map(entry_run_id)
        .map(str::to_owned)
        .collect();
    let mut preamble_dropped = 0u64;
    for (index, entry) in entries.iter().enumerate() {
        // The session record is never part of a turn and never dropped.
        if is_session_record(entry) {
            selected.push(entry.clone());
            continue;
        }
        let turn = turn_of(index, &starts);
        let keep = if selectors.restricts_turns() {
            turn.is_some_and(|turn| kept[turn])
        } else {
            true
        };
        if keep {
            selected.push(entry.clone());
            continue;
        }
        if selectors.restricts_turns() && turn.is_none() {
            preamble_dropped += 1;
        }
        if let Some(run) = entry_run_id(entry) {
            affected_runs.insert(run.to_owned());
        }
    }

    // Removing a tool call must remove the result that answers it: an orphan
    // `tool` message makes every later request fail (the API requires a
    // preceding message with tool_calls). The reverse direction is left to the
    // existing load-time repair, which materialises a "tool execution lost"
    // placeholder — the notes say so rather than silently rewriting assistants.
    let mut removed_call_ids: HashSet<String> = HashSet::new();
    if selectors.drop.contains(&DropKind::ToolCalls) {
        for entry in &selected {
            for block in content_blocks(entry) {
                if block_kind(&block) == "tool_call" {
                    if let Some(id) = block.get("id").and_then(serde_json::Value::as_str) {
                        removed_call_ids.insert(id.to_string());
                    }
                }
            }
            for call in &entry.tool_calls {
                if !call.id.is_empty() {
                    removed_call_ids.insert(call.id.clone());
                }
            }
        }
    }

    let mut kinds = selectors.drop.clone();
    // A checkpoint is a projection over the turns it covers, so a turn-scoped
    // reset drops it: keeping it would feed the summary of dropped turns — and
    // its `protected_entry_ids` — back into the next request.
    let checkpoint_by_scope = selectors.restricts_turns() && !kinds.contains(&DropKind::Compaction);
    if checkpoint_by_scope {
        kinds.push(DropKind::Compaction);
    }

    let mut effects: Vec<DropEffect> = Vec::new();
    let mut paired_results = 0u64;
    let mut paired_blocks = 0u64;
    let mut retained: Vec<SessionEntry> = Vec::with_capacity(selected.len());
    for mut entry in selected {
        if !removed_call_ids.is_empty()
            && entry.entry_type == ENTRY_TYPE_TOOL
            && entry_answers_call(&entry).is_some_and(|call| removed_call_ids.contains(&call))
        {
            paired_results += 1;
            paired_blocks += content_blocks(&entry).len() as u64;
            if let Some(run) = entry_run_id(&entry) {
                affected_runs.insert(run.to_owned());
            }
            continue;
        }
        let mut killed_by: Option<(DropKind, u64)> = None;
        let mut changed = false;
        for kind in &kinds {
            let (blocks, survives) = drop_kind_from_entry(&mut entry, *kind);
            if !survives {
                killed_by = Some((*kind, blocks));
                break;
            }
            if blocks > 0 {
                changed = true;
                record_effect(&mut effects, *kind, 1, blocks);
            }
        }
        if let Some((kind, blocks)) = killed_by {
            record_effect(&mut effects, kind, 1, blocks);
            if let Some(run) = entry_run_id(&entry) {
                affected_runs.insert(run.to_owned());
            }
            continue;
        }
        if changed {
            if let Some(run) = entry_run_id(&entry) {
                affected_runs.insert(run.to_owned());
            }
        }
        retained.push(entry);
    }
    record_effect(
        &mut effects,
        DropKind::ToolResults,
        paired_results,
        paired_blocks,
    );

    // A run that disappeared entirely has no journal left to keep.
    let after_runs: BTreeSet<String> = retained
        .iter()
        .filter_map(entry_run_id)
        .map(str::to_owned)
        .collect();
    for run in before_runs.difference(&after_runs) {
        affected_runs.insert(run.clone());
    }

    if retained.is_empty() {
        bail!(
            "nothing would be left: this session has no session record entry to keep, and an \
             empty transcript cannot be loaded again"
        );
    }

    let tokens_after = estimate_entries_tokens(&retained);
    let turns_kept = kept.iter().filter(|flag| **flag).count() as u64;
    let mut notes = Vec::new();
    let checkpoints_dropped = effects
        .iter()
        .find(|effect| effect.kind == DropKind::Compaction.name())
        .is_some_and(|effect| effect.entries > 0);
    if checkpoints_dropped && checkpoint_by_scope {
        notes.push(
            "compaction checkpoints cover turns this reset drops, so they are removed with them"
                .to_string(),
        );
    }
    if preamble_dropped > 0 {
        notes.push(format!(
            "{preamble_dropped} entry(ies) before the first user turn are not part of any turn and were removed with the scope"
        ));
    }
    if paired_results > 0 {
        notes.push(format!(
            "{paired_results} tool result(s) were removed with the calls they answered, because an orphaned result makes every later request fail"
        ));
    }
    if selectors.drop.contains(&DropKind::ToolResults)
        && !selectors.drop.contains(&DropKind::ToolCalls)
    {
        notes.push(
            "kept tool calls without their results load as a \"tool execution lost\" placeholder (the existing repair path)"
                .to_string(),
        );
    }
    effects.sort_by(|a, b| a.kind.cmp(&b.kind));
    let plan = ResetPlan {
        entries_before: entries.len() as u64,
        entries_after: retained.len() as u64,
        turns_before: turns as u64,
        turns_kept,
        estimated_tokens_before: before_tokens,
        estimated_tokens_after: tokens_after,
        drop_effects: effects,
        first_kept: retained.first().map(preview_of),
        last_kept: retained.last().map(preview_of),
        notes,
    };
    Ok(ResetPlanResult {
        plan,
        retained,
        affected_runs,
    })
}

#[cfg(test)]
#[path = "reset_tests.rs"]
mod tests;

#[cfg(test)]
#[allow(clippy::type_complexity)]
static RESET_RACE_HOOKS: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashMap<String, Box<dyn Fn(&super::Manager) + Send>>>,
> = std::sync::LazyLock::new(Default::default);

/// Test-only hook fired by `reset_history` in the window between the digest it
/// captured and the rewrite transaction that re-reads the history — the one
/// place a concurrent writer can slip in. It makes that race deterministic
/// instead of hoping for one, so the refusal path is tested rather than assumed.
///
/// Keyed by session: four tests register one, and a single slot let whichever ran
/// last replace the others' registration. The reset then found no hook for its
/// own id and succeeded where the test expected `history_changed` — a failure
/// that showed up only at high test parallelism (measured 2026-10-29: red with 32
/// threads, green with 8). The id was always meant to gate the hook; now the
/// storage agrees.
#[cfg(test)]
pub(crate) fn set_reset_race_hook(session: &str, hook: Box<dyn Fn(&super::Manager) + Send>) {
    RESET_RACE_HOOKS.lock().insert(session.to_string(), hook);
}

/// Result of one reset call: the plan that was computed (and, when applied, the
/// plan that was written), plus where the lossless backup went.
#[derive(Debug)]
pub struct ResetOutcome {
    pub plan: ResetPlan,
    pub applied: bool,
    pub backup: Option<std::path::PathBuf>,
}

impl super::Manager {
    /// Plan, and optionally apply, an in-place history reset.
    ///
    /// The session record is never rewritten: selectors only ever remove entries,
    /// so id, name, cwd, model, permission, prompt segments and context files are
    /// untouched by construction. A dry run touches nothing at all; an apply
    /// writes a lossless backup of the *complete* entry list (including what it
    /// drops) before the rewrite, so a wrong call is recoverable.
    pub fn reset_history(
        &self,
        id: &str,
        selectors: &ResetSelectors,
        apply: bool,
    ) -> Result<ResetOutcome> {
        let store = self.storage()?;
        let canonical = store.storage_format(id)? == super::content::FORMAT_VERSION;
        let revision = store
            .revision(id)?
            .ok_or_else(|| anyhow::anyhow!("reset session is unavailable"))?;
        let raw = store.entries(id)?;
        let entries: Vec<SessionEntry> = raw
            .iter()
            .map(|value| serde_json::from_value(value.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let planned = plan_reset(&entries, selectors)?;
        if !apply {
            return Ok(ResetOutcome {
                plan: planned.plan,
                applied: false,
                backup: None,
            });
        }
        let digest = if canonical {
            None
        } else {
            Some(super::compaction_ops::canonical_hash(
                &serde_json::Value::Array(raw.clone()),
            )?)
        };
        #[cfg(test)]
        {
            // Id-gated so unrelated resets never consume the hook, and taken
            // out of the map before it runs: the hook writes through the same
            // manager, and holding the registry across that work would
            // serialize every other test's reset.
            let hook = RESET_RACE_HOOKS.lock().remove(id);
            if let Some(hook) = hook {
                hook(self);
            }
        }
        let backup = self.write_reset_backup(id, selectors, &planned.plan, &raw)?;
        let plan = if canonical {
            store.apply_segmented_reset(id, revision, &raw, planned)?
        } else {
            store.reset_transcript(
                id,
                digest.as_deref().expect("legacy reset digest"),
                selectors,
            )?
        };
        // The display projection cache is keyed by transcript revision, which
        // the rewrite bumped; drop it anyway so a reader that captured the
        // session's revision before the reset cannot reuse the old projection.
        self.invalidate_display_projection(id);
        Ok(ResetOutcome {
            plan,
            applied: true,
            backup: Some(backup),
        })
    }

    /// Write the pre-reset entry list to a JSON file next to the database.
    ///
    /// The `entries` array is exactly the storage representation of the whole
    /// session, so a future `--restore` is "read this array and write it back" —
    /// nothing is summarised or projected on the way in. Only the write itself is
    /// best-effort atomic (temp file + rename): a half-written backup must never
    /// look like a usable one.
    fn write_reset_backup(
        &self,
        id: &str,
        selectors: &ResetSelectors,
        plan: &ResetPlan,
        entries: &[serde_json::Value],
    ) -> Result<std::path::PathBuf> {
        let directory = self
            .database_path()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("session-backups");
        std::fs::create_dir_all(&directory)
            .map_err(|error| anyhow::anyhow!("create {}: {error}", directory.display()))?;
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
        let path = directory.join(format!("{id}-{stamp}-{}.json", uuid::Uuid::new_v4()));
        let backup = serde_json::json!({
            "format": "futureos-session-backup",
            "version": 1,
            "sessionId": id,
            "createdAt": chrono::Utc::now().to_rfc3339(),
            "reason": "session-history-reset",
            "selectors": selectors.to_json(),
            "entriesBefore": plan.entries_before,
            "entriesAfter": plan.entries_after,
            "entries": entries,
        });
        let mut temporary = tempfile::Builder::new()
            .prefix(".history-reset-")
            .tempfile_in(&directory)
            .map_err(|error| anyhow::anyhow!("create reset backup: {error}"))?;
        serde_json::to_writer_pretty(temporary.as_file_mut(), &backup)
            .map_err(|error| anyhow::anyhow!("encode reset backup: {error}"))?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(&path)
            .map_err(|error| anyhow::anyhow!("publish reset backup: {error}"))?;
        #[cfg(unix)]
        std::fs::File::open(&directory)?.sync_all()?;
        Ok(path)
    }
}
