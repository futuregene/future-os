//! All-14-capabilities contract test: every domain pack in the catalog
//! registers, describes itself, and produces finite typed proposals on
//! marker inputs. Deterministic.

use future_loop::capabilities::{CapabilityRegistry, ProposalKind};

#[test]
fn all_14_capabilities_registered() {
    let r = CapabilityRegistry::with_builtin();
    let names: Vec<&str> = r.all().iter().map(|c| c.name()).collect();
    for expected in [
        "agent_turn_recall",
        "auto_research",
        "change_quality",
        "content_ops",
        "context_providers",
        "decision_context",
        "explore",
        "integration_branch",
        "issue_fix",
        "material_lifecycle",
        "periodic_report",
        "reward_memory",
        "semantic_preference",
        "value_connectors",
    ] {
        assert!(names.contains(&expected), "missing capability {expected}");
        assert!(r.get(expected).is_some(), "registry lookup for {expected}");
    }
    assert_eq!(r.all().len(), 14);
}

#[test]
fn every_capability_proposes_without_panicking() {
    let r = CapabilityRegistry::with_builtin();
    for cap in r.all() {
        for input in [
            "",
            "decide whether to ship the report",
            "research question?",
            "prefer short summaries",
        ] {
            let proposals = cap.propose(input);
            assert!(
                !proposals.is_empty(),
                "{} returned no proposals",
                cap.name()
            );
            for p in &proposals {
                // Finite kinds only; successors carry a todo; gates carry a question.
                match p.kind {
                    ProposalKind::SuccessorTodo | ProposalKind::Repair | ProposalKind::Monitor => {
                        assert!(p.todo.is_some(), "{} successor lacks todo", cap.name());
                    }
                    ProposalKind::Gate => {
                        assert!(
                            p.gate_question.is_some(),
                            "{} gate lacks question",
                            cap.name()
                        );
                    }
                    ProposalKind::NoFollowUp => {}
                }
            }
        }
    }
}

#[test]
fn auto_research_proposes_full_chain() {
    let r = CapabilityRegistry::with_builtin();
    let proposals = r
        .get("auto_research")
        .unwrap()
        .propose("Does feature X improve latency?");
    assert_eq!(proposals.len(), 3, "hypothesis -> execute -> evaluate");
    assert!(proposals
        .iter()
        .all(|p| p.kind == ProposalKind::SuccessorTodo));
    assert!(proposals[0].reason.contains("hypothesis"));
    assert!(proposals[2].reason.contains("evaluate"));
}

#[test]
fn reward_memory_and_preference_gate() {
    let r = CapabilityRegistry::with_builtin();
    let reward = r
        .get("reward_memory")
        .unwrap()
        .propose("reward: good run, keep the approach");
    assert_eq!(reward[0].kind, ProposalKind::Gate);
    assert!(reward[0].gate_question.clone().unwrap().contains("Confirm"));

    let pref = r
        .get("semantic_preference")
        .unwrap()
        .propose("prefer terse changelogs");
    assert_eq!(pref[0].kind, ProposalKind::Gate);
}

#[test]
fn empty_inputs_no_followup() {
    let r = CapabilityRegistry::with_builtin();
    for name in [
        "agent_turn_recall",
        "content_ops",
        "periodic_report",
        "auto_research",
        "explore",
    ] {
        let p = r.get(name).unwrap().propose("");
        assert_eq!(
            p[0].kind,
            ProposalKind::NoFollowUp,
            "{name} must no-follow-up on empty input"
        );
    }
}
