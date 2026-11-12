//! Turn envelope (G-9) — the per-turn prompt assembly, expanded from the P0
//! ~30-line `compose_turn_message` into an envelope: instruction + context +
//! evidence (reference `control_plane/quota/turn_envelope.py`, 796 lines — we
//! implement the minimal deterministic core the host needs).
//!
//! The envelope is a plain-text prompt for a policy-free agent: one bounded
//! turn, with evidence/contracts/signals but no strategy policy (strategy is
//! the agent's responsibility; the kernel only enforces consistency floors).
//! [`compose_turn_message`] keeps the P0 signature used by the gRPC executor
//! and delegates to [`compose_turn_envelope`]. The kernel's scheduling
//! internals (the should-run verdict, mode, reason, arbitration) are
//! deliberately NOT in the envelope — they are addressed to the orchestrator
//! and operator, not the worker (ARCHITECTURE.md "Not in the envelope").

use crate::decision::truncate;
use crate::state::{FailureKind, Goal, RunRecord, TaskClass, Todo, TodoStatus};

/// Recompute the todo's advisory signals from the ledger, using the SAME
/// kernel detectors the delivery reason uses (shared text in
/// `decision::stall::todo_signals`) — the envelope's advisory channel.
/// Exported for the envelope; not a decision: the reading agent acts (or not).
fn compose_todo_signals(goal: &Goal, todo: &Todo) -> Vec<String> {
    crate::decision::todo_signals(goal, todo)
}

/// Envelope schema version (reference `TURN_ENVELOPE_SCHEMA_VERSION`).
/// `future_goal_` is a frozen wire prefix predating the loop → goal rename.
pub const TURN_ENVELOPE_SCHEMA_VERSION: &str = "future_goal_turn_envelope_v0";

/// How many recent semantic events surface in the goal-memory block.
const GOAL_MEMORY_SEMANTIC_EVENTS: usize = 5;
/// Cap the failure-cause text so a runaway error never bloats the prompt.
const GOAL_MEMORY_ERROR_CHARS: usize = 200;

/// The failure-cause line for the envelope. A runaway narration error is capped,
/// but an acceptance rejection is NOT: it is a short machine-written string whose
/// point is the trailing `(missing: [...])` (or the "truncated before acceptance"
/// byte offsets), and a 200-char cap cut a four-token list down to
/// `(missing: [dispatches_…` — the worker read a fragment and re-guessed.
fn failure_cause_text(error: &str) -> String {
    if error.contains("acceptance contract unmet") {
        error.to_string()
    } else {
        truncate(error, GOAL_MEMORY_ERROR_CHARS)
    }
}
/// Summary byte budget shared across all upstream sources. Source index entries
/// are retained separately, even when a wide fan-in exhausts summary space.
const UPSTREAM_EVIDENCE_CHARS: usize = 1_200;

/// Compose the goal-memory block fed to the orchestrating agent every turn.
///
/// This closes the gap between what the kernel *collects* (failure
/// classification, semantic history) and what the agent *actually sees*:
///
/// 1. the current todo's failure context — how many prior attempts failed and
///    WHY (classified: infra-recoverable vs verify-gate vs hard error), so the
///    agent knows whether to retry, fix the `--verify` gate, or supersede;
/// 2. the goal's recent semantic history — the last few turn outcomes, so a
///    long-run goal's agent stops re-deriving context it already established.
///
/// The kernel still never decides for the agent: it only surfaces
/// observations (ARCHITECTURE-SIMPLIFICATION). Returns an empty string when
/// there is nothing to surface.
pub fn compose_goal_memory(goal: &Goal, todo: &Todo) -> String {
    let mut blocks: Vec<String> = vec![];

    // 1. Current todo failure context (most relevant to THIS turn).
    let failures: Vec<&RunRecord> = goal
        .history
        .iter()
        .filter(|r| r.todo_id == todo.id && run_failed(r))
        .collect();
    if !failures.is_empty() {
        let last = failures.last().unwrap();
        let mut line = format!(
            "prior attempts on this todo: {} failed; last = {}",
            failures.len(),
            failure_label(last)
        );
        if let Some(err) = &last.error {
            line.push_str(&format!(": {}", failure_cause_text(err)));
        } else if let Some(v) = &last.validation {
            if !v.ok {
                line.push_str(&format!(
                    ": verify gate {} rejected (exit {}): {}",
                    v.validator_kind,
                    v.exit_code.unwrap_or(-1),
                    crate::executor::truncate_evidence(&v.summary, 1600)
                ));
            }
        }
        if let Some(cmd) = &todo.validator {
            line.push_str(&format!("; --verify {cmd} must exit 0 to complete"));
        }
        blocks.push(line);
    }

    // 2. Recent semantic history (goal-level, newest at the bottom).
    if !goal.semantic_history.is_empty() {
        let mut lines: Vec<String> = goal
            .semantic_history
            .iter()
            .rev()
            .take(GOAL_MEMORY_SEMANTIC_EVENTS)
            .map(|e| match &e.todo_id {
                Some(id) => format!("- {} [{}] {}", e.kind, id, e.summary),
                None => format!("- {} {}", e.kind, e.summary),
            })
            .collect();
        lines.reverse();
        blocks.push(format!("recent goal history:\n{}", lines.join("\n")));
    }

    if blocks.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("\nPrior activity:\n");
    for b in blocks {
        out.push_str(&b);
        out.push('\n');
    }
    out
}

/// A turn counts as failed when its writeback classification is not
/// `None`; legacy records without a classification fall back to
/// `terminal_state != "completed"`.
fn run_failed(r: &RunRecord) -> bool {
    match r.failure_kind {
        Some(FailureKind::None) => false,
        Some(_) => true,
        None => r.terminal_state != "completed",
    }
}

/// Human-readable failure label for the last attempt (writeback classification
/// first, then a legacy fallback).
fn failure_label(r: &RunRecord) -> String {
    match r.failure_kind {
        Some(kind) => kind.label().to_string(),
        None => format!("legacy `{}` (unclassified)", r.terminal_state),
    }
}

/// Byte cap on a scout's own `kill_when` text in the envelope, so a verbose
/// termination condition cannot bloat the prompt.
const SCOUT_KILL_WHEN_CHARS: usize = 240;

/// Byte budget for the scout contract's FIXED text (everything except the
/// scout's own capped `kill_when`). The block is fixed prose plus one command
/// line, so it is bounded by construction; the budget is what keeps a later
/// edit from quietly turning the exploration contract into a wall of text
/// (enforced by `compose_scout_contract`, and asserted by
/// `scout_contract_stays_within_its_budget`).
pub const SCOUT_CONTRACT_MAX_CHARS: usize = 1_500;

/// Byte cap on the declared scope echoed into the exploration contract. Two
/// ordinary paths fit; a directory-heavy declaration is truncated rather than
/// allowed to compete with the contract itself for the prompt.
const SCOUT_SCOPE_BYTES: usize = 160;

/// Is this todo an exploration node? A scout's product is a direction
/// proposal, so it gets the exploration contract and is spared the delivery
/// contract (acceptance literals, successor declaration) — the two do not
/// coexist.
fn is_exploring(todo: &Todo) -> bool {
    todo.class == TaskClass::Scout
}

/// The scout contract paragraph — what a scout turn must produce, including the
/// exact command that records it.
///
/// A scout is an exploration node (`TaskClass::Scout`), so its output is a
/// direction proposal, not a delivered artifact. The rest of the envelope is
/// written for delivery work (acceptance literals, verify gates, successor
/// declaration), so without this block a scout worker reads a delivery
/// contract and reports an unfinished implementation as failure — and it would
/// have to GUESS the CLI syntax of the one step that matters. Empty for every
/// other class. Fixed text (bounded by [`SCOUT_CONTRACT_MAX_CHARS`]) plus the
/// todo's own `kill_when` (capped), in the same style as the other blocks here.
///
/// `read_scope` is the turn's declared workspace. The contract states it as a
/// READ scope rather than a write set because that is the constraint 50 tool
/// calls of unrequested implementation violate: a scout proposes, so the only
/// writes it may make are evidence notes, and only inside the declared paths.
/// Stating it here (instead of only in the envelope's write-set paragraph)
/// means the constraint and the command that satisfies it arrive together.
pub fn compose_scout_contract(goal: &Goal, todo: &Todo, read_scope: &[String]) -> String {
    if !is_exploring(todo) {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("\nYou are EXPLORING, not delivering. The product of this turn is a direction proposal, not a finished artifact.\n");
    out.push_str("READ-ONLY turn: do not edit existing files or build the implementation — the proposal ledger is your only write; evidence notes are the only files.\n");
    if read_scope.is_empty() {
        out.push_str("Read scope: none declared — make no file writes at all.\n");
    } else {
        out.push_str(&format!(
            "Read scope for this turn (read it, never modify it): {}\n",
            capped_scope(read_scope)
        ));
    }
    out.push_str("Record it with one command (candidate ids are positional: c1, c2, …):\n");
    out.push_str(&format!(
        "  future goal direction propose --goal {} --scout-todo {} --trigger \"<stall|oscillation|gate_decision|…>\" \\\n",
        goal.goal_id, todo.id
    ));
    out.push_str("      --question \"<what this turn settles>\" --observation \"<what you actually saw>\" \\\n");
    out.push_str(
        "      --evidence <path> --candidate \"title|class|relation|kill_when|acceptance1;acceptance2\"\n",
    );
    out.push_str("Fields 2–4: class is `scout` (a deeper exploration — kill_when, the observation that drops it, is REQUIRED) or `advancement` (kill_when may be empty; acceptance carries the contract); the route is `fork` or `revise:<TODO_ID>`; at least one acceptance item is always required.\n");
    out.push_str("A candidate without a kill_when is a guess, not a proposal. Answer the question you were given; do not pick a winner the evidence does not settle.\n");
    out.push_str(&format!(
        "Completion gate: at least one direction proposal naming this scout must be recorded before it is completed — `future goal direction list --goal {}` shows what is already there.\n",
        goal.goal_id
    ));
    // The prompt is re-sent on every scout turn: an edit that spends more of it
    // than the budget is a defect here, not something to discover in a log.
    debug_assert!(
        out.len() <= SCOUT_CONTRACT_MAX_CHARS,
        "the exploration contract grew to {} bytes (budget {SCOUT_CONTRACT_MAX_CHARS}); trim it before shipping",
        out.len()
    );
    if let Some(kill_when) = &todo.kill_when {
        out.push_str(&format!(
            "Stop condition for this scout (kill_when): {}\n",
            truncate(kill_when, SCOUT_KILL_WHEN_CHARS)
        ));
    }
    out
}

/// The declared scope as one bounded line: joined, byte-capped on a char
/// boundary. A declaration is documentation, not a filesystem sandbox, so a
/// path the contract cannot show in full is still enforced by the guard — only
/// the prompt text is truncated.
fn capped_scope(scopes: &[String]) -> String {
    let joined = scopes
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    if joined.len() <= SCOUT_SCOPE_BYTES {
        return joined;
    }
    let mut end = SCOUT_SCOPE_BYTES;
    while end > 0 && !joined.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &joined[..end])
}

/// Compose the per-turn packet: todo + resolved gate decisions + upstream
/// evidence + prior evidence (+ signals when available).///
/// P0 signature (executor + worker paths) — delegates to the full envelope
/// so existing call sites keep working.
pub fn compose_turn_message(goal: &Goal, todo: &Todo, prev: Option<&RunRecord>) -> String {
    compose_turn_envelope(goal, todo, prev)
}

/// Full envelope: schema header + goal context + instruction + resolved gate
/// decisions + upstream evidence (fan-in) + prior evidence + goal memory +
/// signals + completion-contract footer.
///
/// The kernel's scheduling internals (should-run verdict / mode / arbitration)
/// are deliberately absent: they are the kernel's own decision state, addressed
/// to the orchestrator and operator, not to the worker (ARCHITECTURE.md "Not
/// in the envelope"). The OBSERVABLE SIGNALS (outcome floor, oscillation,
/// failure count, no-progress) are different in kind: they are observations
/// about the work, computed from the ledger, and the advisory channel the
/// architecture promises ("surfaces them as advisories in the turn envelope")
/// — so they ARE recomputed here, from the same kernel detectors the delivery
/// reason uses (no duplicated logic).
pub fn compose_turn_envelope(goal: &Goal, todo: &Todo, prev: Option<&RunRecord>) -> String {
    let mut out = String::new();
    out.push_str(&format!("── {TURN_ENVELOPE_SCHEMA_VERSION} ──\n"));
    out.push_str(&format!(
        "goal: {} | objective: {}\n",
        goal.goal_id, goal.objective
    ));
    out.push_str(&format!(
        "execution: cadence={} spend_rule={}\n",
        goal.execution_profile.cadence, goal.execution_profile.spend_rule
    ));
    out.push('\n');

    // Instruction.
    out.push_str(&format!("TODO {}: {}\n", todo.id, todo.text));
    // The declared workspace is what the scout contract re-states as its READ
    // scope, so it is resolved before the contract and printed ONCE: for an
    // exploration turn the write-set paragraph would contradict the contract's
    // "read it, never modify it" and invite the implementation a scout is not
    // supposed to produce.
    let writer = todo
        .claimed_by
        .as_deref()
        .or(todo.owner.as_deref())
        .unwrap_or("");
    let scopes = crate::agents::workspace_guard::todo_workspaces(goal, writer, todo);
    // Exploration turns get their own output contract (empty for non-scouts).
    out.push_str(&compose_scout_contract(goal, todo, &scopes));
    if !scopes.is_empty() && !is_exploring(todo) {
        out.push_str(&format!(
            "Declared write set (coordination contract, not a filesystem sandbox): {}\nStay within these paths, including logs/caches; if additional writes are needed, stop and ask the supervisor to revise the plan.\n",
            scopes.join(", ")
        ));
    }

    if let Some(acceptance) = &todo.acceptance {
        // A scout is judged by its proposal ledger, not by delivery literals:
        // the acceptance paragraph (and its "last N bytes" note) belongs to
        // delivery work only. Keeping it for a scout made the envelope
        // self-contradictory — "you are exploring" immediately followed by
        // "your completion must contain each of those strings verbatim".
        if !is_exploring(todo) {
            out.push_str(&format!(
                "Acceptance contract (do not silently weaken): {acceptance}\n"
            ));
            // Only the tail of the completion text is judged (see
            // `completion::EVIDENCE_TAIL_BYTES`), so an acceptance line written
            // earlier in the reply is invisible to the gate — say so BEFORE the
            // attempt instead of rejecting it afterwards.
            out.push_str(&format!(
                "The final section of your completion must contain each of those strings verbatim — they must land inside the last {} bytes of your reply, which is all the acceptance gate reads.\n",
                crate::completion::EVIDENCE_TAIL_BYTES
            ));
        }
    }
    if let Some(validator) = &todo.validator {
        out.push_str(&format!("Machine verification: {validator}\nA manual completion is a reviewed override, not a machine pass.\n"));
    }
    // Context: resolved gate decisions flow into blocked todos' packets.
    if let Some(gate_ids) = todo.blocked_by_gate.as_deref() {
        let decisions: Vec<String> = gate_ids
            .split(',')
            .filter_map(|gid| goal.todo(gid))
            .filter(|g| g.status == TodoStatus::Done)
            .filter_map(|g| g.decision.clone().map(|d| format!("{}: {}", g.id, d)))
            .collect();
        if !decisions.is_empty() {
            out.push_str(&format!(
                "\nResolved gate decision(s): {}\n",
                decisions.join("; ")
            ));
        }
    }

    // Upstream evidence: a fan-in (synthesis) todo ordered after its
    // done/superseded predecessors carries their evidence summaries.
    out.push_str(&compose_upstream_evidence(goal, todo));

    // Evidence from the previous turn.
    if let Some(p) = prev {
        out.push_str(&format!(
            "\nEvidence from the previous turn (todo {}):\n{}",
            p.todo_id,
            truncate(&p.evidence, 1_200)
        ));
    }

    // Goal memory: prior failures on this todo + recent semantic history.
    out.push_str(&compose_goal_memory(goal, todo));

    // Signals: observations about THIS todo's work, recomputed from the
    // ledger with the same kernel detectors the delivery reason uses
    // (outcome floor / oscillation / failure count / no-progress). The
    // advisory channel the architecture promises; what to do about a signal
    // stays the worker's decision.
    let signals = compose_todo_signals(goal, todo);
    if !signals.is_empty() {
        out.push_str(&format!("\nsignals: {}\n", signals.join(" ")));
    }
    // Completion contract footer (LoopX: completion must declare closure intent).
    out.push_str("\n\nComplete the todo and report what you did and observed.");
    if !is_exploring(todo) {
        // Delivery closure: an advancement turn must say what comes next. A
        // scout's closure intent is its proposal ledger (stated above), so the
        // successor sentence — and `--no-follow-up` with it — would only ask it
        // to declare work it is not qualified to sequence.
        out.push_str("\nOn completion, declare the successor todo or --no-follow-up.");
    }
    out.push_str("\nHandoff: identify artifacts and paths, new evidence versus the previous attempt, rejected approaches, uncertainties and the next useful check. Activity alone is not progress; budget exhaustion is not successful completion.");
    out
}

/// Compose the upstream-evidence block for a fan-in (synthesis) todo.
///
/// A synthesis todo is ordered after its upstream predecessors by `--blocks`
/// (each predecessor id sits in `blocked_by_gate`); its envelope injects each
/// done/superseded predecessor's completion evidence so the synthesis worker
/// reads what landed instead of re-deriving it (ARCHITECTURE.md
/// "Worker↔worker: … its envelope injects their evidence and artifact paths").
///
/// The combined summary is capped at [`UPSTREAM_EVIDENCE_CHARS`]. Returns an
/// empty string when there are no done/superseded predecessors with evidence.
pub fn compose_upstream_evidence(goal: &Goal, todo: &Todo) -> String {
    let Some(ids) = todo.blocked_by_gate.as_deref() else {
        return String::new();
    };
    let predecessors: Vec<&Todo> = ids
        .split(',')
        .map(str::trim)
        .filter_map(|id| goal.todo(id))
        .filter(|pred| matches!(pred.status, TodoStatus::Done | TodoStatus::Superseded))
        .collect();
    // Every predecessor gets an index entry; summaries share the budget fairly.
    // The index is O(fan-in), intentionally never silently dropping a source.
    let per_source = UPSTREAM_EVIDENCE_CHARS / predecessors.len().max(1);
    let mut entries: Vec<String> = Vec::new();
    for pred in predecessors {
        let evidence = pred
            .evidence
            .as_deref()
            .unwrap_or("[no evidence recorded]")
            .trim();
        let snippet = if per_source >= 12 {
            crate::executor::truncate_evidence(evidence, per_source)
        } else {
            String::new()
        };
        entries.push(format!(
            "upstream {}: {} [status={:?}]",
            pred.id, snippet, pred.status
        ));
    }
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("\nUpstream evidence: (summaries only; read referenced artifacts, not just these snippets)\n");
    out.push_str(&format!("Full source evidence: future goal status --goal {} --format json (source IDs below). Superseded sources are not verified results.\n", goal.goal_id));
    for e in entries {
        out.push_str(&e);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Todo;
    use std::time::Duration;

    /// A scout is an exploration node: its envelope states the proposal
    /// contract — INCLUDING the exact command and candidate syntax, so the
    /// worker never guesses — and carries the scout's own stop condition.
    ///
    /// The scout block REPLACES the delivery contract rather than stacking on
    /// top of it: no acceptance-literals paragraph (there is no delivery gate
    /// for a scout) and no "declare the successor todo" sentence. An
    /// advancement turn keeps both, so the replacement is exact — not a
    /// general loosening of the delivery envelope.
    #[test]
    fn scout_envelope_states_the_proposal_contract_and_advancement_does_not() {
        let mut g = Goal::new("g1", "o", "/tmp");
        let mut scout = Todo::advancement("S1", "Find the right approach");
        scout.class = TaskClass::Scout;
        scout.kill_when = Some("two candidates evaluated against the fixture".into());
        // Accepted candidates carry acceptance criteria; a scout must not be
        // handed the literals-verbatim contract for them.
        scout.acceptance = Some("hit-rate measured,latency p50 recorded".into());
        g.add(scout);
        let mut work = Todo::advancement("T1", "Do the work");
        work.acceptance = Some("report.md landed".into());
        g.add(work);

        let scout_msg = compose_turn_message(&g, g.todo("S1").unwrap(), None);
        assert!(
            scout_msg.contains("You are EXPLORING, not delivering."),
            "scout: {scout_msg}"
        );
        // The executable step, spelled out with the real CLI surface.
        assert!(
            scout_msg.contains("future goal direction propose --goal g1 --scout-todo S1"),
            "the recording command must be exact: {scout_msg}"
        );
        assert!(
            scout_msg.contains("\"title|class|relation|kill_when|acceptance1;acceptance2\""),
            "the candidate syntax must be exact: {scout_msg}"
        );
        assert!(
            scout_msg.contains("`revise:<TODO_ID>`"),
            "the revise relation must be named: {scout_msg}"
        );
        assert!(
            scout_msg.contains("class is `scout`")
                && scout_msg.contains("`advancement`")
                && scout_msg.contains("is REQUIRED"),
            "the class rule (scout needs kill_when) must be stated: {scout_msg}"
        );
        // The completion gate is stated up front, not discovered on rejection.
        assert!(
            scout_msg
                .contains("at least one direction proposal naming this scout must be recorded"),
            "the proposal gate must be stated: {scout_msg}"
        );
        assert!(
            scout_msg.contains("two candidates evaluated against the fixture"),
            "the scout's own stop condition must reach the worker: {scout_msg}"
        );
        // ...and NOT the delivery contract it replaces.
        assert!(
            !scout_msg.contains("Acceptance contract"),
            "a scout is not judged on delivery literals: {scout_msg}"
        );
        assert!(
            !scout_msg.contains("must contain each of those strings verbatim"),
            "the acceptance gate is not part of an exploration turn: {scout_msg}"
        );
        assert!(
            !scout_msg.contains("declare the successor todo")
                && !scout_msg.contains("--no-follow-up"),
            "a scout does not declare successors: {scout_msg}"
        );

        let adv_msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(
            !adv_msg.contains("You are EXPLORING, not delivering."),
            "advancement: {adv_msg}"
        );
        assert!(!adv_msg.contains("candidates"), "advancement: {adv_msg}");
        assert!(
            !adv_msg.contains("direction propose"),
            "advancement: {adv_msg}"
        );
        // Still the ordinary delivery contract, unchanged.
        assert!(adv_msg.contains("Acceptance contract (do not silently weaken): report.md landed"));
        assert!(adv_msg.contains("must contain each of those strings verbatim"));
        assert!(adv_msg.contains("Complete the todo and report what you did and observed."));
        assert!(adv_msg.contains("declare the successor todo or --no-follow-up"));
    }

    /// The exploration contract is bounded text plus one capped `kill_when`: a
    /// later edit must not grow it into a wall of prose (the prompt is spent on
    /// every scout turn). Measured with realistic id lengths, since the ids are
    /// part of the command line.
    #[test]
    fn scout_contract_stays_within_its_budget() {
        let mut g = Goal::new("goal_0123456789ab", "o", "/tmp");
        let mut scout = Todo::advancement("todo_0123456789ab", "probe");
        scout.class = TaskClass::Scout;
        scout.kill_when = Some("x".repeat(SCOUT_KILL_WHEN_CHARS * 4));
        g.add(scout);
        let todo = g.todo("todo_0123456789ab").unwrap();
        // Both the bare contract and the one carrying a long (truncated) scope
        // must fit: the scope is caller-supplied text, so its cap is part of the
        // budget rather than an assumption about short paths.
        for scope in [
            vec![],
            vec![
                "a-rather-long-directory-name/nested/deeper/and-deeper/evidence-one/".to_string(),
                "another-long-directory/with/a/long/suffix/evidence-two/".to_string(),
            ],
        ] {
            let block = compose_scout_contract(&g, todo, &scope);
            let stop_line = block
                .lines()
                .find(|line| line.starts_with("Stop condition for this scout"))
                .expect("the capped stop condition is still emitted")
                .len();
            let fixed = block.len() - stop_line;
            assert!(
                fixed <= SCOUT_CONTRACT_MAX_CHARS,
                "the fixed exploration contract is {fixed} bytes with scope {scope:?}, over its {SCOUT_CONTRACT_MAX_CHARS}-byte budget"
            );
        }
    }

    /// The exploration contract is a READ-ONLY contract with a named scope:
    /// the failure it prevents is a scout turn that spends its budget building
    /// the implementation instead of proposing a direction. Both constraints
    /// must be stated mechanically, and the delivery write-set paragraph must
    /// not reappear to contradict them.
    #[test]
    fn scout_contract_states_read_only_and_its_read_scope() {
        let mut g = Goal::new("g1", "o", "/tmp");
        let mut scout = Todo::advancement("S1", "Find the right approach");
        scout.class = TaskClass::Scout;
        scout.required_write_scope = vec!["papers/route-a.md".to_string()];
        g.add(scout);
        let msg = compose_turn_message(&g, g.todo("S1").unwrap(), None);
        assert!(msg.contains("READ-ONLY"), "{msg}");
        assert!(
            msg.contains("do not edit existing files"),
            "the read-only constraint must be spelled out: {msg}"
        );
        assert!(
            msg.contains("Read scope for this turn (read it, never modify it): ")
                && msg.contains("papers/route-a.md"),
            "the declared scope must reach the contract as a read scope: {msg}"
        );
        assert!(
            !msg.contains("Declared write set"),
            "an exploration turn must not also be handed a write set: {msg}"
        );

        // No declaration: the contract still closes the write surface instead of
        // leaving it to the model's judgement.
        let mut bare = Todo::advancement("S2", "probe");
        bare.class = TaskClass::Scout;
        g.add(bare);
        let bare_msg = compose_turn_message(&g, g.todo("S2").unwrap(), None);
        assert!(
            bare_msg.contains("Read scope: none declared — make no file writes at all."),
            "{bare_msg}"
        );

        // Delivery work keeps the ordinary write set.
        let mut work = Todo::advancement("T1", "Do the work");
        work.required_write_scope = vec!["papers/final.md".to_string()];
        g.add(work);
        let work_msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(work_msg.contains("Declared write set"), "{work_msg}");
        assert!(!work_msg.contains("READ-ONLY"), "{work_msg}");
        assert!(
            !compose_scout_contract(&g, g.todo("T1").unwrap(), &[]).contains("READ-ONLY"),
            "the scout contract is empty for delivery work"
        );
    }

    /// A scout without a declared `kill_when` (old ledgers) still gets the
    /// contract, without a dangling "Stop condition" line, and a runaway
    /// kill_when is capped instead of bloating the prompt.
    #[test]
    fn scout_contract_handles_a_missing_or_runaway_kill_when() {
        let mut g = Goal::new("g1", "o", "/tmp");
        let mut bare = Todo::advancement("S1", "probe");
        bare.class = TaskClass::Scout;
        g.add(bare);
        let msg = compose_turn_message(&g, g.todo("S1").unwrap(), None);
        assert!(msg.contains("You are EXPLORING, not delivering."), "{msg}");
        assert!(!msg.contains("Stop condition for this scout"), "{msg}");

        let mut runaway = Todo::advancement("S2", "probe");
        runaway.class = TaskClass::Scout;
        runaway.kill_when = Some("x".repeat(4_000));
        g.add(runaway);
        let msg = compose_turn_message(&g, g.todo("S2").unwrap(), None);
        assert!(
            !msg.contains(&"x".repeat(SCOUT_KILL_WHEN_CHARS + 1)),
            "the stop condition must be capped: {} bytes",
            msg.len()
        );
    }

    #[test]
    fn envelope_carries_schema_and_instruction() {
        let mut g = Goal::new("g1", "Ship the thing", "/tmp");
        g.add(Todo::advancement("T1", "Do the work"));
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(msg.contains(TURN_ENVELOPE_SCHEMA_VERSION));
        assert!(msg.contains("TODO T1: Do the work"));
        assert!(msg.contains("Complete the todo and report what you did and observed."));
        assert!(
            msg.contains("--no-follow-up"),
            "completion contract footer present"
        );
        assert!(msg.contains("objective: Ship the thing"));
    }

    /// B: the acceptance rejection must reach the worker WHOLE. A four-token
    /// list used to be cut at 200 chars to `(missing: [dequant_gemm_small_n,
    /// matmul2d_int4…`, so the worker read a fragment of its own contract.
    /// C: the envelope must also say the literal has to sit in the judged tail
    /// BEFORE the attempt, not explain it after a rejection.
    #[test]
    fn acceptance_rejection_arrives_whole_and_the_tail_window_is_stated_up_front() {
        use crate::state::{FailureKind, RunRecord};
        let mut g = Goal::new("g", "o", "/tmp");
        let mut todo = Todo::advancement("T1", "Work");
        todo.acceptance = Some(
            "dequant_gemm_small_n,matmul2d_int4b_executed,mlx_comparison,lever_status_updated"
                .into(),
        );
        g.add(todo);
        let rejection = "todo T1 acceptance contract unmet: evidence must contain [dequant_gemm_small_n,matmul2d_int4b_executed,mlx_comparison,lever_status_updated] (missing: [dequant_gemm_small_n, matmul2d_int4b_executed, mlx_comparison, lever_status_updated]); repair the handoff, not the criteria";
        g.history.push(RunRecord {
            agent_id: None,
            turn: 1,
            todo_id: "T1".to_string(),
            run_id: "run-1".to_string(),
            terminal_state: "completed".to_string(),
            error: Some(rejection.to_string()),
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: "prose".to_string(),
            recorded_at: 0,
            spend_source: Some("run".to_string()),
            failure_kind: Some(FailureKind::HardError),
            validation: None,
            truncation: None,
        });

        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(
            msg.contains(
                "(missing: [dequant_gemm_small_n, matmul2d_int4b_executed, mlx_comparison, lever_status_updated])"
            ),
            "the missing list must arrive complete: {msg}"
        );
        assert!(
            msg.contains("must land inside the last 4000 bytes"),
            "the judged window is stated before the attempt: {msg}"
        );
    }

    #[test]
    fn envelope_includes_the_task_write_contract() {
        let dir = tempfile::tempdir().unwrap();
        let mut goal = Goal::new("g", "work", dir.path().to_str().unwrap());
        let mut todo = Todo::advancement("t", "write report");
        todo.required_write_scope = vec!["papers/a.md".into()];
        goal.add(todo);
        let message = compose_turn_message(&goal, goal.todo("t").unwrap(), None);
        assert!(message.contains("Declared write set"));
        // The contract carries the path the guard resolves (canonicalized
        // parent + relative tail), which is platform-spelled.
        let resolved =
            crate::agents::workspace_guard::normalize_workspace_path_at("papers/a.md", dir.path());
        assert!(message.contains(&resolved), "{message}");
        assert!(message.contains("stop and ask the supervisor"));
    }

    #[test]
    fn envelope_includes_resolved_gate_decisions() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::user_gate("G1", "Approve X", &["T2"]));
        g.todo_mut("G1").unwrap().decision = Some("approved".to_string());
        g.todo_mut("G1").unwrap().status = TodoStatus::Done;
        g.add(Todo::advancement("T2", "Blocked work").blocking(&["G1"]));
        let msg = compose_turn_message(&g, g.todo("T2").unwrap(), None);
        assert!(msg.contains("Resolved gate decision(s): G1: approved"));
    }

    #[test]
    fn envelope_includes_prior_evidence() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        let prev = RunRecord {
            agent_id: None,
            turn: 1,
            todo_id: "T0".to_string(),
            run_id: "run-1".to_string(),
            terminal_state: "completed".to_string(),
            error: None,
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: "previous evidence payload".to_string(),
            recorded_at: 0,
            spend_source: Some("run".to_string()),
            validation: None,
            failure_kind: None,
            truncation: None,
        };
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), Some(&prev));
        assert!(msg.contains("Evidence from the previous turn (todo T0)"));
        assert!(msg.contains("previous evidence payload"));
    }

    #[test]
    fn envelope_omits_kernel_scheduling_internals() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::monitor("M1", "Watch", Duration::from_secs(3600)));
        g.todo_mut("M1").unwrap().resume_when =
            Some(std::time::SystemTime::now() - Duration::from_secs(5));
        let msg = compose_turn_envelope(&g, g.todo("M1").unwrap(), None);
        // The should-run verdict / mode / arbitration are the kernel's own
        // decision state, addressed to the orchestrator — never the worker.
        assert!(!msg.contains("decision:"), "decision: leaked: {msg}");
        assert!(!msg.contains("should_run:"), "should_run: leaked: {msg}");
        assert!(!msg.contains("mode:"), "mode: leaked: {msg}");
        assert!(!msg.contains("arbitration:"), "arbitration: leaked: {msg}");
    }

    #[test]
    fn envelope_carries_signals_as_advisories() {
        // Signals are observations about the work, not kernel decision state:
        // the architecture surfaces them as advisories in the turn envelope,
        // recomputed from the ledger with the same detectors as the delivery
        // reason. This goal has a failed attempt + a breached outcome floor.
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        g.todo_mut("T1").unwrap().failed_attempts = 2;
        g.execution_profile.outcome_floor_streak_threshold = 3;
        g.outcome_streak = 4;
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(msg.contains("signals: [signal:"), "envelope: {msg}");
        assert!(
            msg.contains("has 2 failed attempt(s)"),
            "failure-count signal: {msg}"
        );
        assert!(
            msg.contains("outcome floor"),
            "outcome-floor signal recomputed: {msg}"
        );
        // ...and the same detector text the decision kernel emits:
        let packet = crate::decision::decide(&g, std::time::SystemTime::now());
        assert!(packet.reason.contains("has 2 failed attempt(s)"));
        assert!(packet.reason.contains("outcome floor"));
    }

    #[test]
    fn envelope_has_no_signals_block_when_clean() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(!msg.contains("signals:"), "envelope: {msg}");
    }

    #[test]
    fn envelope_injects_upstream_evidence_for_fan_in() {
        let mut g = Goal::new("g1", "o", "/tmp");
        let mut up1 = Todo::advancement("U1", "Probe direction A");
        up1.status = TodoStatus::Done;
        up1.evidence = Some("direction A landed: found X".to_string());
        let mut up2 = Todo::advancement("U2", "Probe direction B");
        up2.status = TodoStatus::Done;
        up2.evidence = Some("direction B landed: found Y".to_string());
        g.add(up1);
        g.add(up2);
        g.add(Todo::advancement("S1", "Synthesize the probes").blocking(&["U1", "U2"]));
        let msg = compose_turn_message(&g, g.todo("S1").unwrap(), None);
        assert!(msg.contains("Upstream evidence:"), "envelope: {msg}");
        assert!(
            msg.contains("upstream U1: direction A landed: found X"),
            "envelope: {msg}"
        );
        assert!(
            msg.contains("upstream U2: direction B landed: found Y"),
            "envelope: {msg}"
        );
    }

    fn run_rec(
        todo_id: &str,
        terminal_state: &str,
        failure_kind: FailureKind,
        error: Option<&str>,
        validation: Option<crate::state::TaskValidation>,
    ) -> RunRecord {
        RunRecord {
            agent_id: None,
            turn: 1,
            todo_id: todo_id.to_string(),
            run_id: format!("run-{todo_id}"),
            terminal_state: terminal_state.to_string(),
            error: error.map(str::to_string),
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: vec![],
            evidence: String::new(),
            recorded_at: 0,
            spend_source: Some("run".to_string()),
            validation,
            failure_kind: Some(failure_kind),
            truncation: None,
        }
    }

    #[test]
    fn goal_memory_surfaces_prior_failures_with_classification() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        g.history.push(run_rec(
            "T1",
            "error",
            FailureKind::HardError,
            Some("validator exited 1"),
            None,
        ));
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(msg.contains("Prior activity:"), "envelope: {msg}");
        assert!(
            msg.contains("prior attempts on this todo: 1 failed"),
            "envelope: {msg}"
        );
        assert!(msg.contains("hard error"), "envelope: {msg}");
        assert!(msg.contains("validator exited 1"), "envelope: {msg}");
    }

    #[test]
    fn goal_memory_surfaces_verify_gate_rejection() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        g.todo_mut("T1").unwrap().validator = Some("sh -c test".to_string());
        g.history.push(run_rec(
            "T1",
            "completed",
            FailureKind::ScienceVerifyFailed,
            None,
            Some(crate::state::task_validation_receipt(
                crate::state::ValidationStatus::Failed,
                "sh -c test",
                "failed",
                None,
                Some(1),
            )),
        ));
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(
            msg.contains("verify-gate rejected the output"),
            "envelope: {msg}"
        );
        assert!(
            msg.contains("verify gate sh -c test rejected"),
            "envelope: {msg}"
        );
        assert!(
            msg.contains("--verify sh -c test must exit 0 to complete"),
            "envelope: {msg}"
        );
    }

    #[test]
    fn goal_memory_surfaces_semantic_history() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        g.record_semantic_event("todo_completed", Some("T0"), "finished the setup", 1);
        g.record_semantic_event("run_landed", Some("T1"), "started T1", 2);
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(msg.contains("recent goal history:"), "envelope: {msg}");
        assert!(
            msg.contains("todo_completed [T0] finished the setup"),
            "envelope: {msg}"
        );
        assert!(
            msg.contains("run_landed [T1] started T1"),
            "envelope: {msg}"
        );
    }

    #[test]
    fn goal_memory_empty_when_nothing_to_surface() {
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        let msg = compose_turn_message(&g, g.todo("T1").unwrap(), None);
        assert!(!msg.contains("Prior activity:"), "envelope: {msg}");
    }

    /// Legacy records predate the writeback classification: a record with no
    /// `failure_kind` is judged by `terminal_state`, and one explicitly
    /// classified `None` is a success even if its state string is not
    /// `completed`. Getting this wrong either hides a failure or resurrects it.
    #[test]
    fn goal_memory_handles_legacy_and_classified_failure_records() {
        // Legacy: no classification, non-completed state → counted as failed.
        let mut legacy = run_rec("T1", "failed", FailureKind::HardError, None, None);
        legacy.failure_kind = None;
        // Explicitly classified as succeeded → NOT a failure, whatever the state.
        let mut classified_success = run_rec("T1", "stalled", FailureKind::HardError, None, None);
        classified_success.failure_kind = Some(FailureKind::None);
        // Classified as a science failure while the state string says completed.
        let classified_failure = run_rec(
            "T1",
            "completed",
            FailureKind::ScienceVerifyFailed,
            None,
            None,
        );

        // The memory renders the LAST failure, so each record is exercised alone.
        let with = |history: Vec<RunRecord>| {
            let mut g = Goal::new("g1", "o", "/tmp");
            g.add(Todo::advancement("T1", "Work"));
            g.history = history;
            compose_goal_memory(&g, g.todo("T1").unwrap())
        };
        let legacy_msg = with(vec![legacy]);
        assert!(
            legacy_msg.contains("legacy `failed` (unclassified)"),
            "legacy record must be labelled by its state: {legacy_msg}"
        );
        let science_msg = with(vec![classified_failure]);
        assert!(
            science_msg.contains("verify-gate rejected the output"),
            "classify by failure_kind, not the state string: {science_msg}"
        );
        assert!(
            with(vec![classified_success]).is_empty(),
            "a record explicitly classified as succeeded must not surface as a failure"
        );
    }

    /// Semantic history is goal-level, so an event that belongs to no todo must
    /// still be rendered (without an empty `[]` placeholder).
    #[test]
    fn goal_memory_renders_semantic_events_without_a_todo_id() {
        use crate::decision::goal_frontier::semantic_history::SemanticEvent;
        let mut g = Goal::new("g1", "o", "/tmp");
        g.add(Todo::advancement("T1", "Work"));
        g.semantic_history = vec![SemanticEvent {
            kind: "replan_acked".into(),
            todo_id: None,
            summary: "goal-level realignment".into(),
            ts: 1,
        }];
        let msg = compose_goal_memory(&g, g.todo("T1").unwrap());
        assert!(
            msg.contains("- replan_acked goal-level realignment"),
            "unscoped event must not print a placeholder id: {msg}"
        );
        assert!(!msg.contains("[]"), "{msg}");
    }

    /// A wide fan-in gets no per-source snippet (the budget per source falls
    /// below the useful minimum) but must still list every source, and a todo
    /// with no upstream at all must produce no block.
    #[test]
    fn upstream_evidence_scales_the_budget_and_handles_an_empty_fan_in() {
        let mut g = Goal::new("g1", "o", "/tmp");
        let ids: Vec<String> = (0..120).map(|n| format!("U{n}")).collect();
        for id in &ids {
            let mut up = Todo::advancement(id, "probe");
            up.status = crate::state::TodoStatus::Done;
            up.evidence = Some("a long probe finding that would be truncated".into());
            g.add(up);
        }
        let mut sink = Todo::advancement("S1", "synthesize");
        let ids_ref: Vec<&str> = ids.iter().map(String::as_str).collect();
        sink = sink.blocking(&ids_ref);
        g.add(sink);
        let msg = compose_turn_message(&g, g.todo("S1").unwrap(), None);
        assert!(msg.contains("Upstream evidence:"), "{msg}");
        for id in ["U0", "U119"] {
            assert!(msg.contains(&format!("upstream {id}:")), "{id}: {msg}");
        }
        assert!(
            !msg.contains("a long probe finding"),
            "a 120-way fan-in must not spend the per-source snippet budget: {msg}"
        );

        // No predecessors at all → no upstream block.
        let mut plain = Goal::new("g2", "o", "/tmp");
        plain.add(Todo::advancement("S2", "synthesize"));
        let msg = compose_turn_message(&plain, plain.todo("S2").unwrap(), None);
        assert!(!msg.contains("Upstream evidence:"), "{msg}");
    }
}
