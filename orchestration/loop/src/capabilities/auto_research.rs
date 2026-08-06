//! auto_research capability (LoopX: auto_research — hypothesis → execute →
//! evaluate research chain).
//!
//! G-25 deepening: the P3 rule stub becomes a structured research pipeline.
//! The input may be a research question (free text) or a `question:` /
//! `hypothesis:` / `method:` block. The capability:
//!
//! - validates the question (must be falsifiable — a testable claim, not a
//!   vague ask);
//! - proposes the hypothesis → execute → evaluate chain;
//! - raises a MONITOR when the experiment is long-running (needs periodic
//!   observation rather than one bounded turn);
//! - asks for a concrete question when the input is not research-shaped.

use super::{monitor_todo, successor_todo, Capability, TypedProposal};

pub struct AutoResearchCapability;

#[derive(Debug, Clone, Default)]
pub struct ResearchInput {
    pub question: String,
    pub hypothesis: String,
    pub method: String,
}

pub fn parse_research_input(input: &str) -> ResearchInput {
    let mut r = ResearchInput::default();
    for line in input.lines() {
        let trimmed = line.trim();
        if let Some((key, value)) = trimmed.split_once(':') {
            match key.trim().to_lowercase().as_str() {
                "question" => r.question = value.trim().to_string(),
                "hypothesis" => r.hypothesis = value.trim().to_string(),
                "method" => r.method = value.trim().to_string(),
                _ => {}
            }
        }
    }
    if r.question.is_empty() {
        r.question = input.trim().to_string();
    }
    r
}

/// A hypothesis is falsifiable when it makes a concrete, testable claim
/// (comparative, quantitative, or mechanism language).
pub fn is_falsifiable_hypothesis(h: &str) -> bool {
    let l = h.to_lowercase();
    (l.contains("if") && (l.contains("then") || l.contains("when")))
        || l.contains("increases")
        || l.contains("decreases")
        || l.contains("correlates")
        || l.contains("differs")
        || l.contains("faster")
        || l.contains("slower")
        || l.contains(">")
        || l.contains("<")
        || l.contains("==")
        || l.contains("reduces")
        || l.contains("outperforms")
        || l.contains("does not")
}

pub fn is_research_question(q: &str) -> bool {
    let q = q.trim();
    q.ends_with('?')
        || q.contains("how does")
        || q.contains("why does")
        || q.contains("是否")
        || q.contains("如何")
        || q.contains("研究")
}

/// The experiment needs a monitor when the method describes long-running or
/// periodic observation (benchmarks, sweeps, long evals, live metrics).
pub fn needs_monitor(method: &str) -> bool {
    let l = method.to_lowercase();
    l.contains("benchmark")
        || l.contains("sweep")
        || l.contains("long-running")
        || l.contains("long running")
        || l.contains("overnight")
        || l.contains("continuous")
        || l.contains("periodic")
        || l.contains("24h")
        || l.contains("48h")
        || l.contains("weekly")
}

impl Capability for AutoResearchCapability {
    fn name(&self) -> &'static str {
        "auto_research"
    }
    fn describe(&self) -> &'static str {
        "hypothesis -> execute -> evaluate research chain with falsifiability checks and experiment monitors"
    }
    fn propose(&self, input: &str) -> Vec<TypedProposal> {
        let r = parse_research_input(input);
        let text = input.trim();
        if text.is_empty() {
            return vec![TypedProposal::no_followup("empty input for auto_research")];
        }
        let question = r.question.trim();
        if question.is_empty() {
            return vec![TypedProposal::no_followup("empty research question")];
        }
        if !is_research_question(question) {
            return vec![TypedProposal::successor(
                successor_todo(
                    "research",
                    "Clarify the research question: a falsifiable question with a concrete comparison or target metric.",
                ),
                "input is not shaped as a research question",
            )];
        }
        let mut proposals = vec![TypedProposal::successor(
            successor_todo(
                "research",
                "Form a concrete falsifiable hypothesis from the research question and state the comparison or metric that would refute it.",
            ),
            if r.hypothesis.is_empty() {
                "hypothesis step (falsifiable claim required)"
            } else if is_falsifiable_hypothesis(&r.hypothesis) {
                "hypothesis step (recorded hypothesis is falsifiable)"
            } else {
                "hypothesis step (recorded hypothesis is NOT falsifiable — sharpen it)"
            },
        )];
        if needs_monitor(&r.method) {
            proposals.push(TypedProposal::monitor(
                monitor_todo(
                    "research",
                    &format!(
                        "Monitor the long-running experiment ({}) and record progress, blockers and intermediate evidence at each poll.",
                        if r.method.is_empty() { "method from the question" } else { &r.method }
                    ),
                    3600,
                ),
                "experiment is long-running — periodic observation, not one bounded turn",
            ));
        } else {
            proposals.push(TypedProposal::successor(
                successor_todo(
                    "research",
                    "Execute the cheapest experiment that can distinguish the hypothesis; record the raw outcome.",
                ),
                "execute step",
            ));
        }
        proposals.push(TypedProposal::successor(
            successor_todo(
                "research",
                "Evaluate the evidence against the hypothesis; promote or retire it with a reason and next-step recommendation.",
            ),
            "evaluate step",
        ));
        proposals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::ProposalKind;

    #[test]
    fn research_question_yields_hypothesis_execute_evaluate() {
        let cap = AutoResearchCapability;
        let proposals = cap.propose(
            "question: Does batching reduce latency?\nhypothesis: if requests are batched then p95 latency decreases\nmethod: run a 24h benchmark sweep",
        );
        let kinds: Vec<ProposalKind> = proposals.iter().map(|p| p.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                ProposalKind::SuccessorTodo,
                ProposalKind::Monitor,
                ProposalKind::SuccessorTodo,
            ]
        );
        assert!(proposals[0].reason.contains("falsifiable"));
        assert!(proposals[1].todo.as_ref().unwrap().text.contains("Monitor"));
    }

    #[test]
    fn non_research_input_is_clarified() {
        let cap = AutoResearchCapability;
        let proposals = cap.propose("please do something");
        assert_eq!(proposals[0].kind, ProposalKind::SuccessorTodo);
        assert!(proposals[0].todo.as_ref().unwrap().text.contains("Clarify"));
    }

    #[test]
    fn short_experiment_uses_bounded_turn() {
        let cap = AutoResearchCapability;
        let proposals = cap.propose(
            "question: Does caching help?\nhypothesis: caching reduces p99\nmethod: run the unit test suite once",
        );
        let kinds: Vec<ProposalKind> = proposals.iter().map(|p| p.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                ProposalKind::SuccessorTodo,
                ProposalKind::SuccessorTodo,
                ProposalKind::SuccessorTodo,
            ]
        );
        assert!(proposals[1].reason.contains("execute"));
    }

    #[test]
    fn falsifiability_heuristics() {
        assert!(is_falsifiable_hypothesis(
            "if we batch then latency decreases"
        ));
        assert!(is_falsifiable_hypothesis("A is faster than B"));
        assert!(is_falsifiable_hypothesis("X does not affect Y"));
        assert!(!is_falsifiable_hypothesis("maybe something changes"));
    }
}
