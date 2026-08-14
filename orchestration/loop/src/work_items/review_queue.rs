//! First-class PR-review work items (the work-item surface of the
//! `pr_review_queue` capability, P2-3).
//!
//! A review item is an **advancement Todo** carrying the review identity:
//!
//! - `action_kind = github_pr_review`
//! - `required_capability = pr_review_queue` (capability-gated runnability)
//! - `task_repository = git:github.com/<owner/repo>`
//! - stable per-goal todo id `pr-review-<number>` (upsert by PR number)
//!
//! Because the item IS a todo, **claim/lease reuse the existing
//! `task_lease` state machine** (claim / renew / release / expire / steal)
//! with the reviewer as the lease owner — no second lease implementation.
//! Verdicts route through the review contract (通过 approve / 驳回
//! request_changes / 再修 rework): publishing a verdict completes the
//! review work item with evidence; a re-review is driven by a new exact
//! head (the queue observation emits a fresh candidate only on material
//! transitions or explicit handled cursors).

use serde::Deserialize;
use serde::Serialize;

use crate::capabilities::pr_review_queue::ReviewVerdict;
use crate::state::{Goal, TaskClass, Todo, TodoStatus};
use crate::work_items::task_lease;

pub const REVIEW_ACTION_KIND: &str = "github_pr_review";
pub const REVIEW_REQUIRED_CAPABILITY: &str = "pr_review_queue";
pub const REVIEW_TASK_REPOSITORY_PREFIX: &str = "git:github.com/";

/// A first-class review item: the typed projection of one PR exact-head
/// review over its backing Todo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub number: u64,
    pub head_oid: String,
    pub repository: Option<String>,
    pub title: String,
    pub url: Option<String>,
}

impl ReviewItem {
    /// `NUMBER@HEAD_OID` when the head is a full 40/64-hex OID.
    pub fn exact_head_key(&self) -> Option<String> {
        let head = self.head_oid.trim().to_lowercase();
        let is_hex_oid =
            (head.len() == 40 || head.len() == 64) && head.chars().all(|c| c.is_ascii_hexdigit());
        if !is_hex_oid {
            return None;
        }
        Some(format!("{}@{head}", self.number))
    }

    /// Stable per-goal todo id: one open review work item per PR number
    /// (a new exact head supersedes the previous one).
    pub fn todo_id(&self) -> String {
        format!("pr-review-{}", self.number)
    }

    /// The task repository token (reference `git:github.com/<owner/repo>`).
    pub fn task_repository(&self) -> Option<String> {
        self.repository
            .as_deref()
            .map(|repo| format!("{REVIEW_TASK_REPOSITORY_PREFIX}{repo}"))
    }

    /// The work-item text: the review task at this exact head.
    pub fn text(&self) -> String {
        format!(
            "[P1] Review PR #{} at exact head {}; read the diff and checks, publish a review state matching the evidence, and route any merge through repository policy.",
            self.number, self.head_oid
        )
    }

    /// Materialize the first-class work item as an advancement Todo.
    pub fn to_todo(&self, id: &str) -> Todo {
        let mut todo = Todo::advancement(id, &self.text());
        // The todo title stays the PR title (short label); the generic
        // "Review PR #N" phrasing already lives in the todo text.
        todo.title = if self.title.trim().is_empty() {
            format!("Review PR #{}", self.number)
        } else {
            self.title.clone()
        };
        todo.action_kind = Some(REVIEW_ACTION_KIND.to_string());
        todo.required_capability = Some(REVIEW_REQUIRED_CAPABILITY.to_string());
        todo.capability_binding_ref = Some(REVIEW_REQUIRED_CAPABILITY.to_string());
        todo.task_repository = self.task_repository();
        todo
    }

    /// Inverse projection: parse a review work item back out of its Todo.
    /// Returns `None` for todos that are not review items.
    pub fn from_todo(todo: &Todo) -> Option<Self> {
        if todo.class != TaskClass::Advancement {
            return None;
        }
        let is_review = todo.action_kind.as_deref() == Some(REVIEW_ACTION_KIND)
            || todo.required_capability.as_deref() == Some(REVIEW_REQUIRED_CAPABILITY);
        if !is_review {
            return None;
        }
        let number = todo
            .id
            .strip_prefix("pr-review-")
            .and_then(|suffix| suffix.parse::<u64>().ok())
            .or_else(|| parse_pr_number(&todo.title))?;
        let head_oid = parse_exact_head(&todo.text)?;
        let repository = todo
            .task_repository
            .as_deref()
            .and_then(|repo| repo.strip_prefix(REVIEW_TASK_REPOSITORY_PREFIX))
            .map(|repo| repo.to_string());
        Some(Self {
            number,
            head_oid,
            // The review item type is GitHub-scoped (git:github.com/...), so
            // the PR URL is deterministic from repository + number.
            url: repository
                .as_ref()
                .map(|repo| format!("https://github.com/{repo}/pull/{number}")),
            repository,
            title: todo.title.clone(),
        })
    }
}

/// Parse `PR #<number>` from a title.
fn parse_pr_number(title: &str) -> Option<u64> {
    let upper = title.to_uppercase();
    let hash = upper.find("PR #")?;
    let rest = &upper[hash + 4..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Parse the `exact head <40|64-hex>` marker from review-item text.
pub fn parse_exact_head(text: &str) -> Option<String> {
    let needle = "exact head ";
    let pos = text.find(needle)?;
    let rest = &text[pos + needle.len()..];
    let token: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    if token.len() == 40 || token.len() == 64 {
        Some(token.to_lowercase())
    } else {
        None
    }
}

/// The verdict + comment recorded on a review todo, encoded into its note
/// (`review_verdict: <key>` / `review_comment: <text>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordedVerdict {
    pub verdict: Option<ReviewVerdict>,
    pub comment: Option<String>,
}

impl RecordedVerdict {
    /// Encode into the todo note (deterministic, parseable back).
    pub fn to_note(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        if let Some(verdict) = self.verdict {
            lines.push(format!("review_verdict: {}", verdict.key()));
        }
        if let Some(comment) = &self.comment {
            for line in comment.lines() {
                lines.push(format!("review_comment: {line}"));
            }
        }
        lines.join("\n")
    }

    /// Parse from a todo note.
    pub fn from_note(note: Option<&str>) -> Self {
        let mut out = Self::default();
        for line in note.unwrap_or_default().lines() {
            if let Some(value) = line.strip_prefix("review_verdict: ") {
                out.verdict = ReviewVerdict::parse(value.trim());
            } else if let Some(value) = line.strip_prefix("review_comment: ") {
                let text = value.to_string();
                out.comment = Some(match &out.comment {
                    Some(existing) => format!("{existing}\n{text}"),
                    None => text,
                });
            }
        }
        out
    }
}

/// The completion evidence for a published verdict.
pub fn verdict_evidence(
    item: &ReviewItem,
    verdict: ReviewVerdict,
    reviewer: Option<&str>,
    comment: Option<&str>,
) -> String {
    let reviewer = reviewer.unwrap_or("reviewer");
    let comment = comment.map(|c| format!(": {c}")).unwrap_or_default();
    format!(
        "review {} ({}) at exact head {}@{} by {reviewer}{comment}",
        verdict.label(),
        verdict.key(),
        item.number,
        item.head_oid
    )
}

/// The review contract transition (③): publishing a verdict completes the
/// review work item with evidence. A re-review after 驳回/再修 is driven by
/// a new exact head — the queue observation re-selects only on material
/// transitions or explicit handled cursors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictOutcome {
    /// 通过 — the review is published and the item is complete.
    Approve,
    /// 驳回 — changes requested; the item completes and re-review waits for
    /// a new exact head.
    RequestChanges,
    /// 再修 — rework requested; same exact-head semantics.
    Rework,
}

/// Apply a verdict to a review todo in memory: records the note + evidence
/// and marks the item done with an explicit no-follow-up (the completion
/// contract of advancement todos). The caller persists the matching store
/// events (TodoUpdated / TodoCompleted).
pub fn apply_verdict(
    todo: &mut Todo,
    verdict: ReviewVerdict,
    reviewer: Option<&str>,
    comment: Option<&str>,
    now: u64,
) -> Result<VerdictOutcome, String> {
    if todo.action_kind.as_deref() != Some(REVIEW_ACTION_KIND)
        && todo.required_capability.as_deref() != Some(REVIEW_REQUIRED_CAPABILITY)
    {
        return Err(format!("todo {} is not a PR review work item", todo.id));
    }
    let item = ReviewItem::from_todo(todo).ok_or_else(|| "review item parse failed".to_string())?;
    todo.note = Some(
        RecordedVerdict {
            verdict: Some(verdict),
            comment: comment.map(|c| c.to_string()),
        }
        .to_note(),
    );
    todo.evidence = Some(verdict_evidence(&item, verdict, reviewer, comment));
    todo.status = TodoStatus::Done;
    todo.no_follow_up = true;
    todo.completed_at = Some(now);
    todo.updated_at = now;
    Ok(match verdict {
        ReviewVerdict::Approve => VerdictOutcome::Approve,
        ReviewVerdict::RequestChanges => VerdictOutcome::RequestChanges,
        ReviewVerdict::Rework => VerdictOutcome::Rework,
    })
}

// ── claim/lease reuse (task_lease over the review todo) ───────────────────

/// Claim a review work item: the existing task-lease state machine with the
/// reviewer as lease owner. Live leases held by another reviewer conflict;
/// the same reviewer re-claiming is idempotent; an expired lease is stolen.
pub fn claim_review(
    goal: &mut Goal,
    item: &ReviewItem,
    reviewer: &str,
    lease_secs: u64,
    now: u64,
) -> Result<task_lease::ClaimOutcome, String> {
    let todo = goal
        .todo_mut(&item.todo_id())
        .ok_or_else(|| format!("review work item {} not found", item.todo_id()))?;
    task_lease::claim(todo, reviewer, lease_secs, now).map_err(|err| format!("{err:#}"))
}

/// Renew a review lease held by `reviewer`.
pub fn renew_review(
    goal: &mut Goal,
    item: &ReviewItem,
    reviewer: &str,
    lease_secs: u64,
    now: u64,
) -> Result<task_lease::LeaseOp, String> {
    let todo = goal
        .todo_mut(&item.todo_id())
        .ok_or_else(|| format!("review work item {} not found", item.todo_id()))?;
    task_lease::renew(todo, reviewer, lease_secs, now).map_err(|err| format!("{err:#}"))
}

/// Release a review lease held by `reviewer`.
pub fn release_review(
    goal: &mut Goal,
    item: &ReviewItem,
    reviewer: &str,
    now: u64,
) -> Result<task_lease::LeaseOp, String> {
    let todo = goal
        .todo_mut(&item.todo_id())
        .ok_or_else(|| format!("review work item {} not found", item.todo_id()))?;
    task_lease::release(todo, reviewer, now).map_err(|err| format!("{err:#}"))
}

/// Record expiry for a review lease (a lapsed lease returns the item to the
/// frontier; a new reviewer may steal it).
pub fn expire_review(
    goal: &mut Goal,
    item: &ReviewItem,
    now: u64,
) -> Result<task_lease::LeaseOp, String> {
    let todo = goal
        .todo_mut(&item.todo_id())
        .ok_or_else(|| format!("review work item {} not found", item.todo_id()))?;
    task_lease::expire(todo, now).map_err(|err| format!("{err:#}"))
}

/// The derived lease state of a review work item.
pub fn review_lease_status(
    goal: &Goal,
    item: &ReviewItem,
    now: u64,
) -> Option<task_lease::LeaseStatus> {
    goal.todo(&item.todo_id())
        .map(|todo| task_lease::lease_status(todo, now))
}

/// The first-class queue projection: the open review work items of a goal.
#[derive(Debug, Clone, Default)]
pub struct ReviewQueue {
    pub repository: Option<String>,
    pub items: Vec<ReviewItem>,
}

impl ReviewQueue {
    /// Project the open review work items of a goal (done reviews are
    /// completed work items, not part of the active queue).
    pub fn from_goal(goal: &Goal) -> Self {
        let mut queue = Self::default();
        for todo in goal.todos.iter() {
            if todo.status != TodoStatus::Open {
                continue;
            }
            let Some(item) = ReviewItem::from_todo(todo) else {
                continue;
            };
            if queue.repository.is_none() {
                queue.repository = item.repository.clone();
            }
            queue.items.push(item);
        }
        queue.items.sort_by_key(|item| item.number);
        queue
    }

    pub fn get(&self, number: u64) -> Option<&ReviewItem> {
        self.items.iter().find(|item| item.number == number)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::now_epoch;

    fn item(number: u64, head: &str) -> ReviewItem {
        ReviewItem {
            number,
            head_oid: head.to_string(),
            repository: Some("owner/repo".to_string()),
            title: format!("PR {number}"),
            url: Some(format!("https://github.com/owner/repo/pull/{number}")),
        }
    }

    #[test]
    fn todo_roundtrip_preserves_the_review_identity() {
        let item = item(7, &"a".repeat(40));
        let todo = item.to_todo("pr-review-7");
        assert_eq!(todo.class, TaskClass::Advancement);
        assert_eq!(todo.action_kind.as_deref(), Some(REVIEW_ACTION_KIND));
        assert_eq!(
            todo.required_capability.as_deref(),
            Some(REVIEW_REQUIRED_CAPABILITY)
        );
        assert_eq!(
            todo.task_repository.as_deref(),
            Some("git:github.com/owner/repo")
        );
        assert_eq!(todo.title, "PR 7");
        let parsed = ReviewItem::from_todo(&todo).unwrap();
        assert_eq!(parsed, item);
        assert_eq!(
            parsed.exact_head_key(),
            Some(format!("7@{}", "a".repeat(40)))
        );
    }

    #[test]
    fn non_review_todos_do_not_project_as_review_items() {
        let todo = Todo::advancement("t1", "regular work");
        assert!(ReviewItem::from_todo(&todo).is_none());
        // a review-kind todo without a parseable head fails closed
        let mut todo = Todo::advancement("pr-review-9", "Review PR #9 at exact head short");
        todo.action_kind = Some(REVIEW_ACTION_KIND.to_string());
        assert!(ReviewItem::from_todo(&todo).is_none());
    }

    #[test]
    fn exact_head_requires_a_full_oid() {
        assert_eq!(
            item(1, &"b".repeat(40)).exact_head_key(),
            Some(format!("1@{}", "b".repeat(40)))
        );
        assert_eq!(item(1, &"b".repeat(64)).exact_head_key().unwrap().len(), 66);
        assert!(item(1, "short").exact_head_key().is_none());
    }

    #[test]
    fn parse_exact_head_scans_review_text() {
        let todo = item(3, &"c".repeat(40)).to_todo("pr-review-3");
        assert_eq!(parse_exact_head(&todo.text), Some("c".repeat(40)));
        assert_eq!(parse_exact_head("no marker here"), None);
    }

    #[test]
    fn recorded_verdict_note_roundtrips() {
        let recorded = RecordedVerdict {
            verdict: Some(ReviewVerdict::RequestChanges),
            comment: Some("blocking: unsafe unwrap\nline 42".to_string()),
        };
        let note = recorded.to_note();
        assert!(note.contains("review_verdict: request_changes"));
        assert!(note.contains("review_comment: blocking: unsafe unwrap"));
        let parsed = RecordedVerdict::from_note(Some(&note));
        assert_eq!(parsed, recorded);
        // no verdict recorded → empty
        assert_eq!(RecordedVerdict::from_note(None), RecordedVerdict::default());
        assert_eq!(
            RecordedVerdict::from_note(Some("something else")),
            RecordedVerdict::default()
        );
    }

    #[test]
    fn verdict_application_completes_the_work_item() {
        let mut todo = item(5, &"d".repeat(40)).to_todo("pr-review-5");
        let now = now_epoch();
        let outcome = apply_verdict(
            &mut todo,
            ReviewVerdict::RequestChanges,
            Some("alice"),
            Some("missing regression test"),
            now,
        )
        .unwrap();
        assert_eq!(outcome, VerdictOutcome::RequestChanges);
        assert_eq!(todo.status, TodoStatus::Done);
        assert!(todo.no_follow_up);
        assert_eq!(todo.completed_at, Some(now));
        let evidence = todo.evidence.as_deref().unwrap();
        assert!(evidence.contains("驳回"));
        assert!(evidence.contains("alice"));
        assert!(evidence.contains("missing regression test"));
        let recorded = RecordedVerdict::from_note(todo.note.as_deref());
        assert_eq!(recorded.verdict, Some(ReviewVerdict::RequestChanges));
    }

    #[test]
    fn verdict_application_rejects_non_review_todos() {
        let mut todo = Todo::advancement("t1", "regular work");
        let err = apply_verdict(&mut todo, ReviewVerdict::Approve, None, None, 1).unwrap_err();
        assert!(err.contains("not a PR review work item"));
    }

    #[test]
    fn claim_review_reuses_the_task_lease_state_machine() {
        let mut goal = Goal::new("g", "obj", "/tmp");
        goal.add(item(7, &"e".repeat(40)).to_todo("pr-review-7"));
        let now = now_epoch();
        let outcome = claim_review(&mut goal, &item(7, &"e".repeat(40)), "alice", 60, now).unwrap();
        assert_eq!(
            outcome,
            task_lease::ClaimOutcome {
                idempotent: false,
                steal: false
            }
        );
        // a live lease held by another reviewer conflicts (task_lease rule)
        let err = claim_review(&mut goal, &item(7, &"e".repeat(40)), "bob", 60, now).unwrap_err();
        assert!(err.contains("another agent"), "{err}");
        // same reviewer re-claiming is idempotent
        let outcome = claim_review(&mut goal, &item(7, &"e".repeat(40)), "alice", 60, now).unwrap();
        assert!(outcome.idempotent);
        // release by the owner clears the lease
        let op = release_review(&mut goal, &item(7, &"e".repeat(40)), "alice", now).unwrap();
        assert_eq!(op, task_lease::LeaseOp::Released { missing: false });
        // release by a non-owner fails
        claim_review(&mut goal, &item(7, &"e".repeat(40)), "alice", 60, now).unwrap();
        assert!(release_review(&mut goal, &item(7, &"e".repeat(40)), "bob", now).is_err());
    }

    #[test]
    fn expired_review_lease_allows_steal() {
        let mut goal = Goal::new("g", "obj", "/tmp");
        goal.add(item(7, &"e".repeat(40)).to_todo("pr-review-7"));
        let now = now_epoch();
        claim_review(&mut goal, &item(7, &"e".repeat(40)), "alice", 60, now).unwrap();
        // after expiry a new reviewer steals the lease
        let outcome =
            claim_review(&mut goal, &item(7, &"e".repeat(40)), "bob", 60, now + 120).unwrap();
        assert_eq!(
            outcome,
            task_lease::ClaimOutcome {
                idempotent: false,
                steal: true
            }
        );
        assert_eq!(
            review_lease_status(&goal, &item(7, &"e".repeat(40)), now + 120).map(|s| {
                matches!(s, task_lease::LeaseStatus::Active { owner, .. } if owner == "bob")
            }),
            Some(true)
        );
    }

    #[test]
    fn queue_projects_open_review_items_only() {
        let mut goal = Goal::new("g", "obj", "/tmp");
        goal.add(item(2, &"f".repeat(40)).to_todo("pr-review-2"));
        goal.add(item(1, &"f".repeat(40)).to_todo("pr-review-1"));
        goal.add(Todo::advancement("t1", "regular work"));
        let mut done = item(3, &"f".repeat(40)).to_todo("pr-review-3");
        done.status = TodoStatus::Done;
        goal.add(done);
        let queue = ReviewQueue::from_goal(&goal);
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.repository.as_deref(), Some("owner/repo"));
        assert_eq!(queue.items[0].number, 1);
        assert_eq!(queue.items[1].number, 2);
        assert_eq!(queue.get(1).unwrap().number, 1);
        assert!(queue.get(3).is_none());
        assert!(ReviewQueue::from_goal(&Goal::new("e", "o", "/tmp")).is_empty());
    }

    #[test]
    fn empty_title_defaults_to_review_pr_number() {
        let mut it = item(9, &"a".repeat(40));
        it.title = String::new();
        assert_eq!(it.to_todo("pr-review-9").title, "Review PR #9");
    }

    #[test]
    fn non_advancement_todo_does_not_project() {
        let todo = Todo::monitor("m1", "watch", std::time::Duration::from_secs(60));
        assert!(ReviewItem::from_todo(&todo).is_none());
    }

    #[test]
    fn from_todo_falls_back_to_title_for_pr_number() {
        let mut todo = item(42, &"a".repeat(40)).to_todo("custom-id");
        // id has no pr-review- prefix → number comes from the title.
        todo.title = "PR #42".to_string();
        let parsed = ReviewItem::from_todo(&todo).unwrap();
        assert_eq!(parsed.number, 42);
        // Title without "PR #" fails closed.
        todo.title = "Renamed".to_string();
        assert!(ReviewItem::from_todo(&todo).is_none());
        // "PR #" without digits fails closed.
        todo.title = "PR #abc".to_string();
        assert!(ReviewItem::from_todo(&todo).is_none());
    }

    #[test]
    fn rework_verdict_maps_to_rework_outcome() {
        let mut todo = item(5, &"d".repeat(40)).to_todo("pr-review-5");
        let outcome = apply_verdict(&mut todo, ReviewVerdict::Rework, None, None, 1).unwrap();
        assert_eq!(outcome, VerdictOutcome::Rework);
    }

    #[test]
    fn renew_and_expire_review_reuse_the_lease_state_machine() {
        let mut goal = Goal::new("g", "obj", "/tmp");
        goal.add(item(7, &"e".repeat(40)).to_todo("pr-review-7"));
        let now = now_epoch();
        claim_review(&mut goal, &item(7, &"e".repeat(40)), "alice", 60, now).unwrap();
        let op = renew_review(&mut goal, &item(7, &"e".repeat(40)), "alice", 120, now).unwrap();
        assert_eq!(op, task_lease::LeaseOp::Renewed);
        assert!(renew_review(&mut goal, &item(7, &"e".repeat(40)), "bob", 120, now).is_err());
        let op = expire_review(&mut goal, &item(7, &"e".repeat(40)), now + 200).unwrap();
        assert_eq!(op, task_lease::LeaseOp::Expired { had_lease: true });
        let op = expire_review(&mut goal, &item(7, &"e".repeat(40)), now + 200).unwrap();
        assert_eq!(op, task_lease::LeaseOp::Expired { had_lease: false });
    }
}
