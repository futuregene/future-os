//! Direction proposal ledger — the exploration node's output (`TaskClass::
//! Scout`, the 探索节点).
//!
//! A scout answers "which way should we go", so what it delivers is NOT an
//! artifact but a [`DirectionProposal`]: a small set of candidate directions,
//! each carrying its own acceptance criteria, rejection condition (`kill_when`)
//! and cost estimate. The operator then accepts, rejects or expires the
//! proposal; that decision is what turns a direction into work (an accepted
//! candidate becomes a todo whose `origin_proposal` names the proposal).
//!
//! Two layers, deliberately separate:
//!
//! - **pure rules** — [`dedupe_key`] and [`validate_candidates`]. No IO, so the
//!   command layer can run every check before a byte reaches the ledger, and
//!   the tests exercise them directly. Cycle detection is NOT reimplemented
//!   here: [`check_candidate_cycles`] builds a probe graph and defers to
//!   [`crate::work_items::task_graph`], the single owner of the graph rules.
//! - **projection** — [`DirectionLedger`], folded from the ledger events
//!   (`crate::store::Event::Direction*`) by
//!   [`crate::store::Store::direction_ledger`]. Those events are
//!   projection-only for goal state: they never mutate the todo kanban, so a
//!   replay of the goal is unaffected by an open proposal.
//!
//! Lifecycle: `Proposed` → `Accepted` | `Rejected { reason }` | `Expired
//! { deadline }`. Only a *live* proposal (proposed or accepted, and still
//! inside its evaluation window) blocks a re-proposal of the same direction —
//! expiration exists precisely to free that capacity, and a rejected direction
//! may be reconsidered with new evidence.
//! [`DirectionLedger::live_proposals`] is that filter; the window closing is
//! derived from the deadline as well as recorded by `direction expire`, so a
//! forgotten proposal does not occupy the capacity forever.
//!
//! A candidate also carries the [`TaskClass`] of the work it becomes
//! ([`Candidate::class`]): `Advancement` turns the accepted direction into
//! implementation work, `Scout` into a deeper exploration (and only then is a
//! `kill_when` required). That field is what keeps the ledger from being a
//! closed loop of explorations that never reaches delivery.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::state::{Goal, TaskClass, Todo, TodoStatus};

/// Length of the hex prefix [`dedupe_key`] keeps (64-bit-ish collision space,
/// short enough to read in a CLI listing).
pub const DEDUPE_KEY_HEX_LEN: usize = 16;

/// Default evaluation window of a proposal when the operator gave no explicit
/// deadline: seven days from `created_at`. Without a default, a proposal nobody
/// touches would occupy its direction forever — the "live" capacity rule would
/// have no exit but a manual `reject`.
pub const DEFAULT_PROPOSAL_TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// Shortest normalized title that may be matched by CONTAINMENT rather than
/// equality. A one- or two-character direction ("go", "CI") appears inside
/// almost every other title, so those must match exactly; from four characters
/// up, "add two-tier cache" inside "add a two-tier cache to the reader" is the
/// same direction re-worded, which is exactly what re-proposal has to catch.
const MIN_CONTAINMENT_LEN: usize = 4;

/// Synthetic todo-id prefix for the probe graph the cycle check builds. Never
/// reaches a real ledger: the probe goal is a clone used for validation only.
const CANDIDATE_PROBE_PREFIX: &str = "__direction_candidate__/";

/// Leading verbs that carry no direction information ("add X" and "X" are the
/// same direction, so they must hash to the same key). Stem-level only: the
/// point is to catch the phrasing drift of a re-proposal, not to stem English.
const VERB_PREFIXES: &[&str] = &[
    "add",
    "adds",
    "adding",
    "build",
    "builds",
    "building",
    "create",
    "creates",
    "creating",
    "fix",
    "fixes",
    "fixing",
    "implement",
    "implements",
    "implementing",
    "implementation",
    "introduce",
    "introduces",
    "introducing",
    "make",
    "makes",
    "making",
    "update",
    "updates",
    "updating",
];

/// How a candidate relates to the existing todo graph.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CandidateRelation {
    /// Continue / change an existing todo rather than opening a new route. The
    /// targeted todo must exist and must not be `superseded` — a superseded
    /// todo records "this route was abandoned", so a direction may not quietly
    /// resurrect it.
    Revise { todo_id: String },
    /// A new direction: there is no existing todo to revise.
    Fork,
}

/// One candidate direction inside a proposal.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    /// Proposal-scoped id (`c1`, `c2`, …). Unique within the proposal; sibling
    /// candidates reference each other through it.
    pub id: String,
    pub title: String,
    /// What accepting this candidate creates. This is the difference between a
    /// direction that becomes WORK and one that becomes another exploration:
    /// `Advancement` (the default, and what an old ledger without the field
    /// deserializes as) produces an implementation todo, `Scout` produces a
    /// deeper exploration todo. `Scout` requires a `kill_when` — an exploration
    /// without a termination condition has no bounded window.
    #[serde(default = "default_candidate_class")]
    pub class: TaskClass,
    pub relation: CandidateRelation,
    /// Conditions that make this direction accepted ("done" for a direction is
    /// a decided contract, not a delivered artifact). At least one item.
    #[serde(default)]
    pub acceptance: Vec<String>,
    /// Predecessors: an existing todo id, or a sibling candidate id. Resolved
    /// through the todo graph, so a cycle here is a cycle there. A sibling
    /// candidate id wins when a todo of the same id also exists.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Required for a `Scout` candidate ("stop exploring this direction when
    /// <X>"); optional for an `Advancement` one, where the termination
    /// condition is the acceptance contract itself.
    #[serde(default)]
    pub kill_when: String,
    #[serde(default)]
    pub est_cost: Option<f64>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// Serde default for [`Candidate::class`]: an old candidate (recorded before
/// the field existed) is a direction that was meant to become work.
fn default_candidate_class() -> TaskClass {
    TaskClass::Advancement
}

impl Candidate {
    /// A candidate skeleton — `acceptance` / `kill_when` still have to be
    /// filled in; validation rejects them empty (a `Scout` candidate always
    /// needs a `kill_when`, an `Advancement` one needs only acceptance).
    pub fn new(id: &str, title: &str, relation: CandidateRelation) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            class: TaskClass::Advancement,
            relation,
            acceptance: vec![],
            depends_on: vec![],
            kill_when: String::new(),
            est_cost: None,
            confidence: None,
        }
    }

    pub fn with_class(mut self, class: TaskClass) -> Self {
        self.class = class;
        self
    }

    pub fn with_acceptance(mut self, acceptance: &[&str]) -> Self {
        self.acceptance = acceptance.iter().map(|a| a.to_string()).collect();
        self
    }

    pub fn with_kill_when(mut self, kill_when: &str) -> Self {
        self.kill_when = kill_when.to_string();
        self
    }

    pub fn depends_on(mut self, deps: &[&str]) -> Self {
        self.depends_on = deps.iter().map(|d| d.to_string()).collect();
        self
    }

    pub fn with_cost(mut self, est_cost: f64) -> Self {
        self.est_cost = Some(est_cost);
        self
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Some(confidence);
        self
    }
}

/// A scout's direction proposal (what `TaskClass::Scout` work delivers).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DirectionProposal {
    pub id: String,
    pub goal_id: String,
    /// The scout todo that produced it (`None` for an operator-initiated
    /// proposal).
    #[serde(default)]
    pub scout_todo: Option<String>,
    /// What fired the exploration (`stall`, `oscillation`, `gate_decision`,
    /// an explicit operator ask, …).
    pub trigger: String,
    /// The question the exploration answers.
    pub question: String,
    /// What the scout observed that makes this a live question.
    pub observation: String,
    /// Pointers supporting the observation (paths, event ids, run ids).
    #[serde(default)]
    pub evidence: Vec<String>,
    pub candidates: Vec<Candidate>,
    /// Idempotency key over the candidate titles — see [`dedupe_key`]. Stored
    /// for audit/listing; validation always recomputes it, so a stale stored
    /// value can never defeat the duplicate check.
    #[serde(default)]
    pub dedupe_key: String,
    /// Explicit evaluation deadline (epoch secs). `None` means
    /// `created_at + DEFAULT_PROPOSAL_TTL_SECS` — see
    /// [`DirectionProposal::effective_deadline`]. Recorded at propose time when
    /// the operator knows how long the question stays open.
    #[serde(default)]
    pub deadline: Option<u64>,
    pub created_at: u64,
}

impl DirectionProposal {
    /// Build a proposal and stamp its [`dedupe_key`].
    pub fn new(id: &str, goal_id: &str, trigger: &str, question: &str, observation: &str) -> Self {
        let mut proposal = Self {
            id: id.to_string(),
            goal_id: goal_id.to_string(),
            scout_todo: None,
            trigger: trigger.to_string(),
            question: question.to_string(),
            observation: observation.to_string(),
            evidence: vec![],
            candidates: vec![],
            dedupe_key: String::new(),
            deadline: None,
            created_at: crate::state::now_epoch(),
        };
        proposal.dedupe_key = dedupe_key(&proposal);
        proposal
    }

    pub fn with_scout_todo(mut self, scout_todo: &str) -> Self {
        self.scout_todo = Some(scout_todo.to_string());
        self
    }

    pub fn with_evidence(mut self, evidence: &[&str]) -> Self {
        self.evidence = evidence.iter().map(|e| e.to_string()).collect();
        self
    }

    /// Stamp an explicit deadline (epoch secs) on the proposal.
    pub fn with_deadline(mut self, deadline: u64) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// The deadline the projection uses: the explicit one when recorded, else
    /// the default window after `created_at`.
    pub fn effective_deadline(&self) -> u64 {
        self.deadline
            .unwrap_or_else(|| self.created_at.saturating_add(DEFAULT_PROPOSAL_TTL_SECS))
    }

    /// Whether the evaluation window has run out at `now` (epoch secs).
    pub fn deadline_passed(&self, now: u64) -> bool {
        now >= self.effective_deadline()
    }

    /// Append a candidate, keeping the stored `dedupe_key` in sync with the
    /// candidate set.
    pub fn with_candidate(mut self, candidate: Candidate) -> Self {
        self.candidates.push(candidate);
        self.dedupe_key = dedupe_key(&self);
        self
    }
}

/// Normalize a direction title for idempotency comparison: lowercase, strip
/// leading `add` / `implement` / `fix`-class verbs, then drop everything that
/// is not alphanumeric — punctuation AND whitespace, so `Wire up the CLI`,
/// `wire-up-the-cli!` and `Implement CLI wireup` collapse onto one key while
/// the words stay in order.
///
/// Unicode-aware: CJK characters are alphanumeric and survive untouched, so a
/// Chinese title normalizes to its characters with punctuation removed (the
/// verb list only strips word-initial verbs of space-separated titles). The
/// FIRST surviving word is never stripped, so a title made only of verbs still
/// normalizes to something non-empty.
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let words: Vec<&str> = lower
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let mut start = 0;
    while start + 1 < words.len() && VERB_PREFIXES.contains(&words[start]) {
        start += 1;
    }
    words[start..].concat()
}

/// Idempotency key of a direction proposal: the normalized candidate titles,
/// sorted (order-independent) and deduped, joined and hashed with SHA-256; the
/// result is the first [`DEDUPE_KEY_HEX_LEN`] hex chars.
///
/// The key covers the *direction set*, not its wording: two proposals that
/// suggest the same candidate titles hash identically, so a re-proposal after
/// a stall is rejected instead of duplicating the exploration.
pub fn dedupe_key(proposal: &DirectionProposal) -> String {
    let mut titles: Vec<String> = proposal
        .candidates
        .iter()
        .map(|candidate| normalize_title(&candidate.title))
        .filter(|title| !title.is_empty())
        .collect();
    titles.sort();
    titles.dedup();
    let digest = Sha256::digest(titles.join("\n").as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.truncate(DEDUPE_KEY_HEX_LEN);
    hex
}

/// Whether two normalized titles name the same direction.
///
/// Equality first; then CONTAINMENT in either direction, which is what catches
/// a re-proposal that only re-worded the title ("two-tier cache" inside "add a
/// two-tier cache to the reader"). Containment needs a
/// [`MIN_CONTAINMENT_LEN`]-character shorter side so a two-letter title cannot
/// swallow every other one.
pub fn titles_collide(left: &str, right: &str) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    let (short, long) = if left.chars().count() <= right.chars().count() {
        (left, right)
    } else {
        (right, left)
    };
    short.chars().count() >= MIN_CONTAINMENT_LEN && long.contains(short)
}

/// Validate a candidate set against the goal WITHOUT the proposal history
/// (the goal-side rules only). This is the form a caller with no ledger handle
/// can use; [`DirectionLedger::validate`] is the complete check and adds the
/// duplicate-proposal rule — see [`validate_candidates_against`] for why the
/// ledger is a parameter.
pub fn validate_candidates(goal: &Goal, proposal: &DirectionProposal) -> Result<(), String> {
    validate_candidates_against(goal, proposal, &[])
}

/// Full candidate validation: `ledger` is the proposals already recorded for
/// this goal (typically [`DirectionLedger::live_proposals`]).
///
/// The goal-side checks:
/// - at least one candidate; a non-empty id and title on each; unique candidate
///   ids; at least one non-blank acceptance item, and a `kill_when` on every
///   `Scout` candidate (`Advancement` candidates may leave it empty: their
///   termination condition is the acceptance contract);
/// - idempotency against existing work — a candidate whose normalized title
///   collides with an existing todo's ([`titles_collide`]: equal, or one
///   contained in the other) is `duplicate direction: <todo id>` (the
///   candidate's own `Revise` target is exempt: revising a todo naturally
///   shares its title);
/// - `depends_on` references an existing todo or a sibling candidate;
/// - `Revise { todo_id }` names an existing, non-`superseded` todo;
/// - no dependency cycle among the candidates, detected by the todo graph
///   ([`check_candidate_cycles`]).
///
/// The ledger-side checks: a proposal whose [`dedupe_key`] matches a proposal
/// already in `ledger` is `duplicate proposal: <proposal id>`; otherwise each
/// candidate title is compared one by one with every candidate of those
/// live proposals, so re-wording a single direction cannot slip past the
/// whole-set hash.
pub fn validate_candidates_against(
    goal: &Goal,
    proposal: &DirectionProposal,
    ledger: &[&DirectionProposal],
) -> Result<(), String> {
    if proposal.candidates.is_empty() {
        return Err("a direction proposal needs at least one candidate".to_string());
    }
    let mut seen_ids: BTreeSet<&str> = BTreeSet::new();
    for candidate in &proposal.candidates {
        if candidate.id.trim().is_empty() {
            return Err("a candidate needs a non-empty id".to_string());
        }
        if !seen_ids.insert(candidate.id.as_str()) {
            return Err(format!("duplicate candidate id `{}`", candidate.id));
        }
    }

    for candidate in &proposal.candidates {
        if candidate.title.trim().is_empty() {
            return Err(format!("candidate `{}` has no title", candidate.id));
        }
        if candidate.class == TaskClass::Scout && candidate.kill_when.trim().is_empty() {
            return Err(format!(
                "candidate `{}` is a scout and has no kill_when (when is this exploration abandoned?)",
                candidate.id
            ));
        }
        if candidate
            .acceptance
            .iter()
            .all(|item| item.trim().is_empty())
        {
            return Err(format!(
                "candidate `{}` has no acceptance criteria",
                candidate.id
            ));
        }
        // A `Revise` candidate is ABOUT its target todo, so that one todo is
        // exempt from the duplicate-title check; every other todo still counts.
        let revise_target = match &candidate.relation {
            CandidateRelation::Revise { todo_id } => Some(todo_id.as_str()),
            CandidateRelation::Fork => None,
        };
        let normalized = normalize_title(&candidate.title);
        if !normalized.is_empty() {
            for todo in &goal.todos {
                if Some(todo.id.as_str()) == revise_target {
                    continue;
                }
                let existing = if todo.title.trim().is_empty() {
                    normalize_title(&todo.text)
                } else {
                    normalize_title(&todo.title)
                };
                if titles_collide(&existing, &normalized) {
                    return Err(format!("duplicate direction: {}", todo.id));
                }
            }
        }
        for dep in &candidate.depends_on {
            if seen_ids.contains(dep.as_str()) || goal.todo(dep).is_some() {
                continue;
            }
            return Err(format!(
                "candidate `{}` depends on unknown todo `{dep}`",
                candidate.id
            ));
        }
        if let CandidateRelation::Revise { todo_id } = &candidate.relation {
            let Some(target) = goal.todo(todo_id) else {
                return Err(format!(
                    "candidate `{}` revises unknown todo `{todo_id}`",
                    candidate.id
                ));
            };
            if target.status == TodoStatus::Superseded {
                return Err(format!(
                    "candidate `{}` cannot revive superseded todo `{todo_id}`",
                    candidate.id
                ));
            }
        }
    }

    // Whole-set first: an identical direction set deserves the shortest answer
    // ("this exploration is already on file"), and only then does the per
    // candidate pass catch the re-worded single direction.
    let key = dedupe_key(proposal);
    for existing in ledger {
        if dedupe_key(existing) == key {
            return Err(format!("duplicate proposal: {}", existing.id));
        }
    }
    for candidate in &proposal.candidates {
        let normalized = normalize_title(&candidate.title);
        if normalized.is_empty() {
            continue;
        }
        for existing in ledger {
            for other in &existing.candidates {
                if titles_collide(&normalize_title(&other.title), &normalized) {
                    return Err(format!(
                        "duplicate direction: candidate `{}` of live proposal {} (\"{}\")",
                        candidate.id, existing.id, other.title
                    ));
                }
            }
        }
    }

    check_candidate_cycles(goal, proposal)
}

/// Reject a candidate set that would introduce a dependency cycle.
///
/// The candidates are projected as probe todos (`depends_on` becomes their
/// `blocked_by_gate` predecessor list — the encoding the todo graph reads), and
/// [`crate::work_items::task_graph::validate_dependency_change`] judges each
/// one. That entry point is the graph module's own "is this new/edited node
/// sound" check: it rejects unknown *and* self references and walks the cycles
/// through the candidate's node. Reusing it keeps ONE cycle algorithm in the
/// crate — this module only feeds the graph, it never walks edges itself.
fn check_candidate_cycles(goal: &Goal, proposal: &DirectionProposal) -> Result<(), String> {
    let candidate_ids: BTreeSet<&str> = proposal
        .candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect();
    let mut probe = goal.clone();
    for candidate in &proposal.candidates {
        let predecessors: Vec<String> = candidate
            .depends_on
            .iter()
            .map(|dep| {
                if candidate_ids.contains(dep.as_str()) {
                    probe_todo_id(dep)
                } else {
                    dep.clone()
                }
            })
            .collect();
        let refs: Vec<&str> = predecessors.iter().map(String::as_str).collect();
        let node =
            Todo::advancement(&probe_todo_id(&candidate.id), &candidate.title).blocking(&refs);
        probe.add(node);
    }
    for candidate in &proposal.candidates {
        crate::work_items::task_graph::validate_dependency_change(
            &probe,
            &probe_todo_id(&candidate.id),
        )
        .map_err(|err| {
            format!(
                "candidate `{}`: {}",
                candidate.id,
                err.replace(CANDIDATE_PROBE_PREFIX, "")
            )
        })?;
    }
    Ok(())
}

fn probe_todo_id(candidate_id: &str) -> String {
    format!("{CANDIDATE_PROBE_PREFIX}{candidate_id}")
}

/// Where a proposal stands. `Proposed` is the only non-terminal state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DirectionStatus {
    Proposed,
    Accepted,
    /// Rejected with the reason the operator recorded (never empty — the event
    /// constructor refuses an empty one).
    Rejected {
        reason: String,
    },
    /// Expired at `deadline` (epoch secs): the exploration ran out of time and
    /// the direction frees up for a later re-proposal.
    Expired {
        deadline: u64,
    },
}

impl DirectionStatus {
    /// Whether the direction still occupies the goal's exploration capacity:
    /// proposed or accepted. A rejected/expired direction may be re-proposed.
    pub fn is_live(&self) -> bool {
        matches!(self, DirectionStatus::Proposed | DirectionStatus::Accepted)
    }
}

/// One proposal plus its current decision.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DirectionEntry {
    pub proposal: DirectionProposal,
    pub status: DirectionStatus,
    /// Epoch secs of the newest ledger event for this proposal.
    pub updated_at: u64,
}

impl DirectionEntry {
    /// The status to RENDER at `now`: a still-open proposal whose evaluation
    /// window has run out reads as `Expired` even when no `DirectionExpired`
    /// event was recorded.
    ///
    /// This is projection-level derivation on purpose — the ledger event stays
    /// the audit record an operator writes with `direction expire`, while a
    /// forgotten proposal must not look live forever just because nobody wrote
    /// it down.
    pub fn effective_status(&self, now: u64) -> DirectionStatus {
        if self.status.is_live() && self.proposal.deadline_passed(now) {
            return DirectionStatus::Expired {
                deadline: self.proposal.effective_deadline(),
            };
        }
        self.status.clone()
    }

    /// Whether [`DirectionEntry::effective_status`] had to derive the expiry
    /// (no event recorded it). Callers use this to say so in a listing.
    pub fn expiry_is_derived(&self, now: u64) -> bool {
        self.status.is_live() && self.proposal.deadline_passed(now)
    }
}

/// The direction read model: every proposal of a goal with its decision,
/// folded from the `direction_*` ledger events (see
/// [`crate::store::Store::direction_ledger`]).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DirectionLedger {
    pub entries: Vec<DirectionEntry>,
}

impl DirectionLedger {
    /// Fold the ledger events. Unknown/unrelated events are skipped, so this is
    /// safe to run over a goal's whole event stream (a projection must never
    /// fail on an event it does not own). A decision event for a proposal this
    /// ledger never saw is ignored — the proposal line is what creates the
    /// entry.
    pub fn from_events<'a>(events: impl IntoIterator<Item = &'a crate::store::Event>) -> Self {
        use crate::store::Event;
        let mut ledger = Self::default();
        for event in events {
            match event {
                Event::DirectionProposed { proposal, ts, .. } => {
                    ledger.record(proposal.clone(), *ts);
                }
                Event::DirectionAccepted {
                    proposal_id, ts, ..
                } => ledger.decide(proposal_id, DirectionStatus::Accepted, *ts),
                Event::DirectionRejected {
                    proposal_id,
                    reason,
                    ts,
                    ..
                } => ledger.decide(
                    proposal_id,
                    DirectionStatus::Rejected {
                        reason: reason.clone(),
                    },
                    *ts,
                ),
                Event::DirectionExpired {
                    proposal_id,
                    deadline,
                    ts,
                    ..
                } => ledger.decide(
                    proposal_id,
                    DirectionStatus::Expired {
                        deadline: *deadline,
                    },
                    *ts,
                ),
                _ => {}
            }
        }
        ledger
    }

    /// Record a proposal (first write wins: replaying the same event twice, or
    /// a duplicate id with different content, must not fork the entry).
    fn record(&mut self, proposal: DirectionProposal, ts: u64) {
        if self.proposal(&proposal.id).is_some() {
            return;
        }
        self.entries.push(DirectionEntry {
            proposal,
            status: DirectionStatus::Proposed,
            updated_at: ts,
        });
    }

    /// Apply a decision to a known proposal (latest wins).
    fn decide(&mut self, proposal_id: &str, status: DirectionStatus, ts: u64) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.proposal.id == proposal_id)
        {
            entry.status = status;
            entry.updated_at = ts;
        }
    }

    pub fn proposal(&self, proposal_id: &str) -> Option<&DirectionEntry> {
        self.entries
            .iter()
            .find(|entry| entry.proposal.id == proposal_id)
    }

    /// Proposals that still occupy capacity (proposed or accepted) AND whose
    /// evaluation window is still open at `now`.
    ///
    /// The deadline half is derived, not recorded: a proposal whose window ran
    /// out frees its direction exactly like a recorded `DirectionExpired`, which
    /// is why `direction expire` writes the event for the audit trail but is not
    /// required for the capacity rule to work. Set `now` to 0 to ignore
    /// deadlines entirely.
    pub fn live_proposals_at(&self, now: u64) -> Vec<&DirectionProposal> {
        self.entries
            .iter()
            .filter(|entry| entry.status.is_live() && !entry.proposal.deadline_passed(now))
            .map(|entry| &entry.proposal)
            .collect()
    }

    /// [`DirectionLedger::live_proposals_at`] at the current time — the set the
    /// duplicate-proposal rule compares against.
    pub fn live_proposals(&self) -> Vec<&DirectionProposal> {
        self.live_proposals_at(crate::state::now_epoch())
    }

    /// Validate a new proposal against the goal AND this ledger (the
    /// duplicate-proposal rule included).
    pub fn validate(&self, goal: &Goal, proposal: &DirectionProposal) -> Result<(), String> {
        validate_candidates_against(goal, proposal, &self.live_proposals())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TodoStatus;

    fn goal_with_todos() -> Goal {
        let mut goal = Goal::new("g1", "objective", "/tmp");
        goal.add(Todo::advancement("t1", "Implement the direction ledger"));
        goal.add(
            Todo::advancement("t2", "Wire the CLI")
                .with_title("Wire the CLI")
                .blocking(&["t1"]),
        );
        let mut superseded = Todo::advancement("t3", "Ship the old route");
        superseded.status = TodoStatus::Superseded;
        goal.add(superseded);
        goal
    }

    fn valid_candidate(id: &str, title: &str) -> Candidate {
        Candidate::new(id, title, CandidateRelation::Fork)
            .with_acceptance(&["proposal recorded"])
            .with_kill_when("two candidates are rejected")
    }

    fn valid_proposal() -> DirectionProposal {
        DirectionProposal::new("p1", "g1", "stall", "which way?", "two stalls in a row")
            .with_scout_todo("s1")
            .with_candidate(valid_candidate("c1", "Parallelize the validator"))
    }

    // ── dedupe_key / normalize_title ──────────────────────────────────────

    #[test]
    fn normalized_titles_drop_verbs_and_punctuation() {
        assert_eq!(
            normalize_title("Implement the Direction-Ledger!"),
            "thedirectionledger"
        );
        assert_eq!(normalize_title("  add   FIX the CLI  "), "thecli");
        // A title made only of verbs keeps its last word (never normalizes to "").
        assert_eq!(normalize_title("Add"), "add");
        // Non-ASCII titles survive the punctuation pass; a leading CJK word is
        // not a verb, so it also stops prefix stripping.
        assert_eq!(
            normalize_title("加 add 方向提案账本。"),
            "加add方向提案账本"
        );
        assert_eq!(normalize_title("方向提案账本，加 CLI"), "方向提案账本加cli");
    }

    #[test]
    fn dedupe_key_is_order_independent_and_wording_insensitive() {
        let a = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Add the direction ledger"))
            .with_candidate(valid_candidate("c2", "Implement CLI wire-up"));
        let b = DirectionProposal::new("p2", "g1", "oscillation", "q2", "o2")
            .with_candidate(valid_candidate("x1", "cli wireup"))
            .with_candidate(valid_candidate("x2", "the Direction-Ledger!"));
        assert_eq!(dedupe_key(&a), dedupe_key(&b));
        assert_eq!(a.dedupe_key, dedupe_key(&a), "constructor stamps the key");
        assert_eq!(dedupe_key(&a).len(), DEDUPE_KEY_HEX_LEN);
        assert!(dedupe_key(&a).chars().all(|c| c.is_ascii_hexdigit()));

        let c = DirectionProposal::new("p3", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "A different direction"));
        assert_ne!(dedupe_key(&a), dedupe_key(&c));
    }

    // ── validate_candidates ───────────────────────────────────────────────

    #[test]
    fn valid_proposal_passes() {
        let goal = goal_with_todos();
        assert!(validate_candidates(&goal, &valid_proposal()).is_ok());
    }

    #[test]
    fn duplicate_direction_against_an_existing_todo_is_rejected() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Implement the Direction Ledger!"));
        let err = validate_candidates(&goal, &proposal).unwrap_err();
        assert_eq!(err, "duplicate direction: t1");
    }

    #[test]
    fn revising_a_todo_with_its_own_title_is_not_a_duplicate() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o").with_candidate(
            Candidate::new(
                "c1",
                "Implement the direction ledger",
                CandidateRelation::Revise {
                    todo_id: "t1".to_string(),
                },
            )
            .with_acceptance(&["revised scope recorded"])
            .with_kill_when("the revision is out of scope"),
        );
        assert!(validate_candidates(&goal, &proposal).is_ok());
    }

    #[test]
    fn duplicate_proposal_in_the_ledger_is_rejected() {
        let goal = goal_with_todos();
        let first = valid_proposal();
        let second = DirectionProposal::new("p2", "g1", "stall", "q again", "o")
            .with_candidate(valid_candidate("c1", "add parallelize THE validator"));
        let ledger = [&first];
        let err = validate_candidates_against(&goal, &second, &ledger).unwrap_err();
        assert_eq!(err, "duplicate proposal: p1");

        // Through the read-model entry point (the caller-facing form).
        let read_model = DirectionLedger {
            entries: vec![DirectionEntry {
                proposal: first,
                status: DirectionStatus::Proposed,
                updated_at: 1,
            }],
        };
        assert_eq!(
            read_model.validate(&goal, &second),
            Err("duplicate proposal: p1".to_string())
        );
    }

    #[test]
    fn expired_or_rejected_proposals_do_not_block_a_re_proposal() {
        let goal = goal_with_todos();
        let first = valid_proposal();
        let second = DirectionProposal::new("p2", "g1", "stall", "q again", "o")
            .with_candidate(valid_candidate("c1", "Parallelize the validator"));
        for status in [
            DirectionStatus::Expired { deadline: 10 },
            DirectionStatus::Rejected {
                reason: "not now".to_string(),
            },
        ] {
            let read_model = DirectionLedger {
                entries: vec![DirectionEntry {
                    proposal: first.clone(),
                    status,
                    updated_at: 1,
                }],
            };
            assert!(
                read_model.validate(&goal, &second).is_ok(),
                "a terminal direction frees the capacity"
            );
        }
        // ... but an ACCEPTED direction still occupies it (it became work).
        let accepted = DirectionLedger {
            entries: vec![DirectionEntry {
                proposal: first,
                status: DirectionStatus::Accepted,
                updated_at: 1,
            }],
        };
        assert_eq!(
            accepted.validate(&goal, &second),
            Err("duplicate proposal: p1".to_string())
        );
    }

    #[test]
    fn candidate_cycle_is_rejected_by_the_todo_graph() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "First route").depends_on(&["c2"]))
            .with_candidate(valid_candidate("c2", "Second route").depends_on(&["c1"]));
        let err = validate_candidates(&goal, &proposal).unwrap_err();
        assert!(
            err.contains("dependency cycle"),
            "cycle must be rejected by task_graph: {err}"
        );
        assert!(err.contains("candidate `c1`") || err.contains("candidate `c2`"));
    }

    #[test]
    fn self_referencing_candidate_is_rejected() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Loop route").depends_on(&["c1"]));
        let err = validate_candidates(&goal, &proposal).unwrap_err();
        assert!(err.contains("cannot reference itself"), "{err}");
    }

    #[test]
    fn candidate_dag_over_existing_todos_is_accepted() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "First route").depends_on(&["t1"]))
            .with_candidate(valid_candidate("c2", "Second route").depends_on(&["c1"]));
        assert!(validate_candidates(&goal, &proposal).is_ok());
    }

    #[test]
    fn reviving_a_superseded_todo_is_rejected() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o").with_candidate(
            Candidate::new(
                "c1",
                "Reopen the old route",
                CandidateRelation::Revise {
                    todo_id: "t3".to_string(),
                },
            )
            .with_acceptance(&["route reopened"])
            .with_kill_when("the route is still out of scope"),
        );
        let err = validate_candidates(&goal, &proposal).unwrap_err();
        assert_eq!(err, "candidate `c1` cannot revive superseded todo `t3`");
    }

    #[test]
    fn revising_an_unknown_todo_is_rejected() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o").with_candidate(
            Candidate::new(
                "c1",
                "Revise nothing",
                CandidateRelation::Revise {
                    todo_id: "missing".to_string(),
                },
            )
            .with_acceptance(&["x"])
            .with_kill_when("y"),
        );
        let err = validate_candidates(&goal, &proposal).unwrap_err();
        assert_eq!(err, "candidate `c1` revises unknown todo `missing`");
    }

    #[test]
    fn unknown_depends_on_is_rejected() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Some route").depends_on(&["missing"]));
        let err = validate_candidates(&goal, &proposal).unwrap_err();
        assert_eq!(err, "candidate `c1` depends on unknown todo `missing`");
    }

    #[test]
    fn scout_candidate_needs_kill_when_but_advancement_does_not() {
        let goal = goal_with_todos();
        // A scout without a termination condition is rejected...
        let scout = Candidate::new("c1", "Route A", CandidateRelation::Fork)
            .with_class(TaskClass::Scout)
            .with_acceptance(&["done"]);
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o").with_candidate(scout);
        assert!(validate_candidates(&goal, &proposal)
            .unwrap_err()
            .contains("is a scout and has no kill_when"));

        // ...an advancement candidate may leave it empty: its termination
        // condition is the acceptance contract.
        let advancement =
            Candidate::new("c1", "Route A", CandidateRelation::Fork).with_acceptance(&["done"]);
        let proposal =
            DirectionProposal::new("p1", "g1", "stall", "q", "o").with_candidate(advancement);
        assert!(validate_candidates(&goal, &proposal).is_ok());
    }

    #[test]
    fn missing_acceptance_and_empty_candidate_set_are_rejected() {
        let goal = goal_with_todos();
        let no_acceptance =
            Candidate::new("c1", "Route A", CandidateRelation::Fork).with_kill_when("budget spent");
        let proposal =
            DirectionProposal::new("p1", "g1", "stall", "q", "o").with_candidate(no_acceptance);
        assert!(validate_candidates(&goal, &proposal)
            .unwrap_err()
            .contains("no acceptance criteria"));

        let empty = DirectionProposal::new("p1", "g1", "stall", "q", "o");
        assert!(validate_candidates(&goal, &empty)
            .unwrap_err()
            .contains("at least one candidate"));
    }

    #[test]
    fn candidate_without_a_title_is_rejected() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "   "));
        assert_eq!(
            validate_candidates(&goal, &proposal).unwrap_err(),
            "candidate `c1` has no title"
        );
    }

    #[test]
    fn duplicate_candidate_ids_are_rejected() {
        let goal = goal_with_todos();
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Route A"))
            .with_candidate(valid_candidate("c1", "Route B"));
        assert_eq!(
            validate_candidates(&goal, &proposal).unwrap_err(),
            "duplicate candidate id `c1`"
        );
    }

    // ── idempotency: re-worded directions ─────────────────────────────────

    #[test]
    fn reworded_direction_is_duplicate_of_an_existing_todo() {
        let goal = goal_with_todos();
        // t1 is "Implement the direction ledger"; the candidate is the same
        // direction with the verb and article moved out of the way. Equality
        // alone missed this, containment catches it.
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Direction ledger"));
        assert_eq!(
            validate_candidates(&goal, &proposal).unwrap_err(),
            "duplicate direction: t1"
        );

        // And the other way round: the candidate is the longer phrasing.
        let longer = DirectionProposal::new("p2", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Wire the CLI to the ledger"));
        assert_eq!(
            validate_candidates(&goal, &longer).unwrap_err(),
            "duplicate direction: t2"
        );
    }

    #[test]
    fn reworded_candidate_of_a_live_proposal_is_rejected() {
        let goal = goal_with_todos();
        let first = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Two-tier cache"));
        // A different candidate SET (so the dedupe_key differs) whose single
        // candidate is the same direction re-worded.
        let second = DirectionProposal::new("p2", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Add a two-tier cache to the reader"));
        let ledger = [&first];
        let err = validate_candidates_against(&goal, &second, &ledger).unwrap_err();
        assert!(
            err.contains("duplicate direction: candidate `c1` of live proposal p1"),
            "got: {err}"
        );

        // A genuinely different direction still passes.
        let third = DirectionProposal::new("p3", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Move the cache to the reader"));
        assert!(validate_candidates_against(&goal, &third, &ledger).is_ok());
    }

    #[test]
    fn very_short_titles_only_match_exactly() {
        // Under the containment floor: a 3-character direction matches only
        // itself, so it cannot swallow every title that happens to mention it.
        assert!(!titles_collide(
            &normalize_title("CLI"),
            &normalize_title("CLI tests")
        ));
        assert!(titles_collide(
            &normalize_title("CLI"),
            &normalize_title("cli!")
        ));
        assert!(titles_collide(
            &normalize_title("Direction-Ledger"),
            &normalize_title("the direction ledger")
        ));
        assert!(titles_collide(
            &normalize_title("the direction ledger"),
            &normalize_title("Direction-Ledger")
        ));
        assert!(!titles_collide("", "anything"));
        // Containment is prefix/suffix-shaped: a word inserted in the MIDDLE
        // is a different key ("wire the cli" vs "wire up the cli"), which is
        // the documented limit of this rule.
        assert!(!titles_collide(
            &normalize_title("Wire the CLI"),
            &normalize_title("Wire up the CLI")
        ));
    }

    // ── deadline / expiry ─────────────────────────────────────────────────

    #[test]
    fn deadline_defaults_to_a_week_and_can_be_overridden() {
        let proposal = DirectionProposal::new("p1", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Route A"));
        assert_eq!(proposal.deadline, None);
        assert_eq!(
            proposal.effective_deadline(),
            proposal.created_at + DEFAULT_PROPOSAL_TTL_SECS
        );
        assert!(!proposal.deadline_passed(proposal.created_at));
        assert!(proposal.deadline_passed(proposal.effective_deadline()));

        let explicit = proposal.clone().with_deadline(1_000);
        assert_eq!(explicit.effective_deadline(), 1_000);
        assert!(explicit.deadline_passed(1_000));
        assert!(!explicit.deadline_passed(999));
    }

    #[test]
    fn an_open_proposal_past_its_deadline_reads_expired_and_frees_capacity() {
        let goal = goal_with_todos();
        let proposed = valid_proposal(); // "Parallelize the validator"
        let ledger = DirectionLedger {
            entries: vec![DirectionEntry {
                proposal: proposed.clone(),
                status: DirectionStatus::Proposed,
                updated_at: 1,
            }],
        };
        let same_direction = DirectionProposal::new("p2", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Parallelize the validator"));

        // Inside the window: live, so the re-proposal is a duplicate.
        let inside = proposed.created_at;
        assert_eq!(ledger.live_proposals_at(inside).len(), 1);
        assert!(ledger.validate(&goal, &same_direction).is_err());
        assert_eq!(
            ledger.entries[0].effective_status(inside),
            DirectionStatus::Proposed
        );

        // Past it: the projection derives `Expired` even though no
        // `DirectionExpired` event was ever written...
        let past = proposed.effective_deadline() + 1;
        assert_eq!(
            ledger.entries[0].effective_status(past),
            DirectionStatus::Expired {
                deadline: proposed.effective_deadline()
            }
        );
        assert!(ledger.entries[0].expiry_is_derived(past));
        // ...and the direction is free again.
        assert!(ledger.live_proposals_at(past).is_empty());
        assert!(validate_candidates_against(
            &goal,
            &same_direction,
            &ledger.live_proposals_at(past)
        )
        .is_ok());
    }

    // ── serde ─────────────────────────────────────────────────────────────

    #[test]
    fn proposal_and_ledger_round_trip_through_serde() {
        let proposal = valid_proposal()
            .with_deadline(4_000)
            .with_candidate(
                Candidate::new(
                    "c2",
                    "Revise the parser",
                    CandidateRelation::Revise {
                        todo_id: "t1".to_string(),
                    },
                )
                .with_acceptance(&["parser revision recorded"])
                .with_kill_when("the parser is out of scope")
                .depends_on(&["c1"])
                .with_cost(1.5)
                .with_confidence(0.4),
            )
            .with_candidate(
                Candidate::new("c3", "Explore the parser", CandidateRelation::Fork)
                    .with_class(TaskClass::Scout)
                    .with_acceptance(&["two routes compared"])
                    .with_kill_when("three parsers evaluated"),
            );
        let json = serde_json::to_string(&proposal).unwrap();
        let back: DirectionProposal = serde_json::from_str(&json).unwrap();
        assert_eq!(back, proposal);
        assert_eq!(back.dedupe_key, dedupe_key(&back));
        assert_eq!(back.deadline, Some(4_000));
        assert_eq!(back.candidates[2].class, TaskClass::Scout);
        assert!(json.contains("\"kind\":\"fork\""));
        assert!(json.contains("\"kind\":\"revise\""));
        assert!(json.contains("\"class\":\"scout\""));

        let ledger = DirectionLedger {
            entries: vec![
                DirectionEntry {
                    proposal: proposal.clone(),
                    status: DirectionStatus::Accepted,
                    updated_at: 7,
                },
                DirectionEntry {
                    proposal: DirectionProposal::new("p2", "g1", "stall", "q", "o")
                        .with_candidate(valid_candidate("c9", "Route Z")),
                    status: DirectionStatus::Rejected {
                        reason: "out of scope".to_string(),
                    },
                    updated_at: 8,
                },
                DirectionEntry {
                    proposal: DirectionProposal::new("p3", "g1", "stall", "q", "o")
                        .with_candidate(valid_candidate("c8", "Route Y")),
                    status: DirectionStatus::Expired { deadline: 99 },
                    updated_at: 9,
                },
            ],
        };
        let json = serde_json::to_string(&ledger).unwrap();
        let back: DirectionLedger = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ledger);
        assert!(json.contains("\"state\":\"rejected\""));
        assert!(json.contains("\"state\":\"expired\""));
    }

    #[test]
    fn a_ledger_recorded_before_the_class_and_deadline_fields_still_reads() {
        // Exactly what a proposal written by the previous version looks like:
        // no `class` on the candidate, no `deadline` on the proposal.
        let json = r#"{
            "id": "p1",
            "goal_id": "g1",
            "trigger": "stall",
            "question": "which way?",
            "observation": "two stalls in a row",
            "candidates": [
                {
                    "id": "c1",
                    "title": "Parallelize the validator",
                    "relation": {"kind": "fork"},
                    "acceptance": ["proposal recorded"],
                    "depends_on": [],
                    "kill_when": "two candidates are rejected"
                }
            ],
            "dedupe_key": "deadbeefdeadbeef",
            "created_at": 1000
        }"#;
        let proposal: DirectionProposal = serde_json::from_str(json).unwrap();
        // Old candidates were directions meant to become work.
        assert_eq!(proposal.candidates[0].class, TaskClass::Advancement);
        assert_eq!(proposal.deadline, None);
        // ...and an old proposal gets the default evaluation window.
        assert_eq!(
            proposal.effective_deadline(),
            1000 + DEFAULT_PROPOSAL_TTL_SECS
        );
    }

    // ── projection over the real ledger ───────────────────────────────────

    #[test]
    fn ledger_folds_store_events_and_goal_replay_ignores_them() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let mut store = crate::store::Store::open(&root).unwrap();
        let goal = goal_with_todos();
        store.register(&goal).unwrap();
        let ts = crate::state::now_epoch();
        store
            .append(crate::store::Event::GoalStarted {
                goal_id: "g1".to_string(),
                ts,
            })
            .unwrap();
        for todo in goal.todos.clone() {
            store
                .append(crate::store::Event::TodoAdded {
                    goal_id: "g1".to_string(),
                    todo,
                    ts,
                })
                .unwrap();
        }

        let accepted = valid_proposal();
        let rejected = DirectionProposal::new("p2", "g1", "oscillation", "q", "o")
            .with_candidate(valid_candidate("c1", "Route Z"));
        let expired = DirectionProposal::new("p3", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Route Y"));
        store
            .append(crate::store::Event::direction_proposed(
                accepted.clone(),
                ts,
            ))
            .unwrap();
        store
            .append(crate::store::Event::direction_proposed(
                rejected.clone(),
                ts,
            ))
            .unwrap();
        store
            .append(crate::store::Event::direction_proposed(expired.clone(), ts))
            .unwrap();
        store
            .append(crate::store::Event::direction_accepted("g1", "p1", ts))
            .unwrap();
        store
            .append(
                crate::store::Event::direction_rejected("g1", "p2", "out of scope", ts).unwrap(),
            )
            .unwrap();
        store
            .append(crate::store::Event::direction_expired(
                "g1",
                "p3",
                ts + 60,
                ts,
            ))
            .unwrap();

        let ledger = store.direction_ledger("g1").unwrap();
        assert_eq!(ledger.entries.len(), 3);
        assert_eq!(
            ledger.proposal("p1").unwrap().status,
            DirectionStatus::Accepted
        );
        assert_eq!(
            ledger.proposal("p2").unwrap().status,
            DirectionStatus::Rejected {
                reason: "out of scope".to_string()
            }
        );
        assert_eq!(
            ledger.proposal("p3").unwrap().status,
            DirectionStatus::Expired { deadline: ts + 60 }
        );
        assert_eq!(ledger.proposal("p1").unwrap().proposal, accepted);
        assert_eq!(ledger.live_proposals().len(), 1);

        // Projection-only: replaying goal state is untouched by proposals.
        let replayed = store.replay("g1").unwrap().unwrap();
        assert_eq!(replayed.todos.len(), goal.todos.len());
        assert!(replayed
            .todos
            .iter()
            .all(|todo| todo.origin_proposal.is_none()));

        // The read model is what a caller validates against.
        let dup = DirectionProposal::new("p9", "g1", "stall", "q", "o")
            .with_candidate(valid_candidate("c1", "Parallelize the validator"));
        assert_eq!(
            ledger.validate(&replayed, &dup),
            Err("duplicate proposal: p1".to_string())
        );
    }

    #[test]
    fn rejected_direction_requires_a_reason() {
        let err = crate::store::Event::direction_rejected("g1", "p1", "  ", 5).unwrap_err();
        assert!(err.to_string().contains("reason"));
        assert!(crate::store::Event::direction_rejected("g1", "p1", "too costly", 5).is_ok());
    }
}
