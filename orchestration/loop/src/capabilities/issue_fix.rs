//! Issue-Fix capability (LoopX: issue-fix — the issue-to-PR product path).
//!
//! G-25 deepening: the P3 rule stub (a few keyword heuristics) becomes a
//! structured pipeline. The input is either free text or a simple key-value
//! issue block (`title:` / `body:` / `repro:` / `error:` / `expected:` /
//! `scope:` / `authority:`); the capability classifies the observation and
//! proposes a FINITE plan:
//!
//! - actionable issue → investigate → fix → validate successor chain;
//! - missing signal → triage (request the missing context);
//! - read-only authority / no write scope → a gate before any fix slice;
//! - repeated failure / regression evidence → a bounded repair;
//! - not suitable → explicit no-follow-up.
//!
//! Propose-only, as always: the kernel decides whether to accept.

use super::{successor_todo, Capability, TypedProposal};

pub struct IssueFixCapability;

/// Parsed issue observation.
#[derive(Debug, Clone, Default)]
pub struct IssueObservation {
    pub title: String,
    pub body: String,
    pub repro: String,
    pub error: String,
    pub expected: String,
    pub scope: String,
    pub authority: String,
    pub has_structured: bool,
}

/// Parse a possibly structured issue input. Accepts `key: value` lines for
/// the known keys; everything else is free text.
pub fn parse_issue(input: &str) -> IssueObservation {
    let mut obs = IssueObservation::default();
    let mut free: Vec<String> = Vec::new();
    for line in input.lines() {
        let trimmed = line.trim();
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim().to_lowercase();
            let value = value.trim();
            match key.as_str() {
                "title" => {
                    obs.title = value.to_string();
                    obs.has_structured = true;
                }
                "body" => {
                    obs.body = value.to_string();
                    obs.has_structured = true;
                }
                "repro" => {
                    obs.repro = value.to_string();
                    obs.has_structured = true;
                }
                "error" => {
                    obs.error = value.to_string();
                    obs.has_structured = true;
                }
                "expected" => {
                    obs.expected = value.to_string();
                    obs.has_structured = true;
                }
                "scope" => {
                    obs.scope = value.to_string();
                    obs.has_structured = true;
                }
                "authority" => {
                    obs.authority = value.to_string();
                    obs.has_structured = true;
                }
                _ => free.push(trimmed.to_string()),
            }
        } else if !trimmed.is_empty() {
            free.push(trimmed.to_string());
        }
    }
    if !obs.has_structured {
        // Free text: reuse the body for heuristic scanning.
        obs.body = free.join(" ");
        if let Some(first) = free.first() {
            obs.title = first.clone();
        }
    }
    obs
}

fn has_repro(obs: &IssueObservation) -> bool {
    let haystack = format!(
        "{} {} {} {} {} {}",
        obs.title, obs.body, obs.repro, obs.error, obs.expected, obs.scope
    )
    .to_lowercase();
    !obs.repro.is_empty()
        || haystack.contains("repro")
        || haystack.contains("steps to reproduce")
        || haystack.contains("minimal example")
}

fn has_error(obs: &IssueObservation) -> bool {
    let haystack = format!(
        "{} {} {} {} {}",
        obs.title, obs.body, obs.repro, obs.error, obs.expected
    )
    .to_lowercase();
    !obs.error.is_empty()
        || haystack.contains("error")
        || haystack.contains("panic")
        || haystack.contains("exception")
        || haystack.contains("stack trace")
        || haystack.contains("crash")
        || haystack.contains("fails")
}

fn has_expected(obs: &IssueObservation) -> bool {
    let haystack = format!("{} {}", obs.title, obs.body).to_lowercase();
    !obs.expected.is_empty()
        || haystack.contains("expected")
        || haystack.contains("should be")
        || haystack.contains("should not")
}

fn write_authority_missing(obs: &IssueObservation) -> bool {
    let a = obs.authority.to_lowercase();
    a.contains("read-only")
        || a.contains("readonly")
        || a.contains("no-write")
        || a.contains("no_write")
        || a.contains("write-scope=none")
}

fn regression_signal(obs: &IssueObservation) -> bool {
    let haystack = format!("{} {} {} {}", obs.title, obs.body, obs.error, obs.scope).to_lowercase();
    haystack.contains("regression")
        || haystack.contains("re-introduced")
        || haystack.contains("broke again")
        || haystack.contains("previously worked")
}

/// Actionability score: how much signal the issue carries (0..3).
pub fn actionability_score(obs: &IssueObservation) -> u32 {
    [has_repro(obs), has_error(obs), has_expected(obs)]
        .iter()
        .filter(|b| **b)
        .count() as u32
}

impl Capability for IssueFixCapability {
    fn name(&self) -> &'static str {
        "issue_fix"
    }
    fn describe(&self) -> &'static str {
        "translate an issue observation into a fix plan (investigate → fix → validate), triage, gate or no-follow-up"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let obs = parse_issue(input);
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("empty issue observation")];
        }
        let score = actionability_score(&obs);
        let word_count = text.split_whitespace().count();

        // Authority gate first: a fix slice is never proposed without write
        // authority — the gate asks the owner to grant scope.
        if write_authority_missing(&obs) {
            return vec![TypedProposal::gate(
                "Grant write scope for the repository before any fix slice is assigned (authority is read-only).",
                "issue is actionable but the lane has no write authority",
            )];
        }

        if regression_signal(&obs) && score >= 2 {
            // Repeated failure: bounded repair, not a fresh plan.
            return vec![
                TypedProposal::successor(
                    successor_todo(
                        "issue",
                        "Repair: bisect the regression to the change that re-introduced the failure and revert or fix it with a repro.",
                    ),
                    "regression signal with repro/error — bounded repair first",
                ),
                TypedProposal::successor(
                    successor_todo(
                        "issue",
                        "Validate the repair with the issue's repro plus the original passing case, then report evidence.",
                    ),
                    "repair validation slice",
                ),
            ];
        }

        if score >= 2 && word_count >= 8 {
            // Actionable: the full investigate → fix → validate plan.
            vec![
                TypedProposal::successor(
                    successor_todo(
                        "issue",
                        &format!(
                            "Reproduce the issue ({}) and locate the root cause; report the failing path with evidence.",
                            if obs.repro.is_empty() {
                                "from the described repro"
                            } else {
                                "using the provided repro"
                            }
                        ),
                    ),
                    "investigate slice: repro + root cause",
                ),
                TypedProposal::successor(
                    successor_todo(
                        "issue",
                        "Implement a focused fix for the root cause and validate it with the repro or a small smoke before reporting.",
                    ),
                    "fix slice",
                ),
                TypedProposal::successor(
                    successor_todo(
                        "issue",
                        "Validate the fix against the expected behavior and run the adjacent test surface; attach evidence.",
                    ),
                    "validate slice",
                ),
            ]
        } else if score == 1 && word_count >= 8 {
            // Partial signal: investigate first, then the fix may follow.
            vec![TypedProposal::successor(
                successor_todo(
                    "issue",
                    "Investigate: gather the missing signal (repro steps, exact error, expected vs actual) and pin the root cause with evidence.",
                ),
                "partial signal — investigate before any fix",
            )]
        } else if word_count < 8 || score == 0 {
            vec![TypedProposal::successor(
                successor_todo(
                    "issue",
                    "Triage: request the missing context (exact error, repro steps, expected vs actual) before acting.",
                ),
                "issue lacks enough signal to act",
            )]
        } else {
            vec![TypedProposal::no_followup(
                "issue is not suitable for a fix PR path",
            )]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::ProposalKind;

    #[test]
    fn structured_issue_yields_full_plan() {
        let cap = IssueFixCapability;
        let proposals = cap.propose(
            "title: crash on empty input\nerror: panicked at src/main.rs:42\nrepro: run with no args\nbody: the tool panics instead of printing usage\nexpected: prints usage and exits 0",
        );
        let kinds: Vec<&str> = proposals
            .iter()
            .map(|p| match p.kind {
                ProposalKind::SuccessorTodo => "successor",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["successor", "successor", "successor"]);
        assert!(proposals[0].reason.contains("investigate"));
        assert!(proposals[1].reason.contains("fix"));
        assert!(proposals[2].reason.contains("validate"));
    }

    #[test]
    fn read_only_authority_gates_before_fix() {
        let cap = IssueFixCapability;
        let proposals =
            cap.propose("title: bug in release\nerror: crash\nauthority: read-only\nrepro: steps");
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, ProposalKind::Gate);
        assert!(proposals[0]
            .gate_question
            .as_deref()
            .unwrap()
            .contains("write scope"));
    }

    #[test]
    fn regression_signal_triggers_repair() {
        let cap = IssueFixCapability;
        let proposals = cap.propose(
            "title: regression\nerror: tests fail\nrepro: run ci\nbody: this previously worked and broke again",
        );
        assert!(proposals[0].reason.contains("repair") || proposals[0].reason.contains("Repair"));
        assert!(proposals[0].todo.as_ref().unwrap().text.contains("bisect"));
    }

    #[test]
    fn weak_signal_triages() {
        let cap = IssueFixCapability;
        let proposals = cap.propose("something is broken");
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        assert!(proposals[0].todo.as_ref().unwrap().text.contains("Triage"));
        // empty input → explicit no-follow-up
        let proposals = cap.propose("   ");
        assert_eq!(proposals[0].kind, ProposalKind::NoFollowUp);
    }

    #[test]
    fn free_text_actionability_scoring() {
        let obs =
            parse_issue("crash with a stack trace; expected it to work; steps to reproduce below");
        assert_eq!(actionability_score(&obs), 3);
        let obs = parse_issue("hi");
        assert_eq!(actionability_score(&obs), 0);
        // structured repro counts even without keywords
        let obs = parse_issue("repro: type 'x' then hit enter");
        assert!(has_repro(&obs));
    }
}
