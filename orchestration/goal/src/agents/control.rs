//! Durable, recipient-scoped steering. Acknowledgement means a successful
//! turn writeback, not merely assembling a prompt. Retrying after an ambiguous
//! disconnect is deliberately at-least-once; instructions must be idempotent.
use crate::store::{Event, Store};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instruction {
    pub id: String,
    pub agent_id: Option<String>,
    pub text: String,
    /// Ordinary guidance waits for the next boundary. Urgent corrections abort.
    pub interrupt: bool,
}

/// Latest instruction per scope (broadcast and this recipient), in ledger
/// order. Acknowledgements by other recipients never consume a broadcast.
pub fn pending(store: &Store, goal: &str, agent: Option<&str>) -> anyhow::Result<Vec<Instruction>> {
    let events = store.events(goal)?;
    let mut instructions: Vec<Instruction> = Vec::new();
    let mut acknowledged = std::collections::HashSet::new();
    for entry in events {
        match entry.event {
            Event::ControlIssued { instruction, .. }
                if instruction.agent_id.is_none() || instruction.agent_id.as_deref() == agent =>
            {
                instructions.retain(|old| old.agent_id != instruction.agent_id);
                instructions.push(instruction);
            }
            Event::ControlAcknowledged {
                instruction_id,
                agent_id,
                ..
            } if agent_id.as_deref() == agent => {
                acknowledged.insert(instruction_id);
            }
            _ => {}
        }
    }
    instructions.retain(|instruction| !acknowledged.contains(&instruction.id));
    Ok(instructions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Goal;

    #[test]
    fn targeted_broadcast_same_second_and_restart_are_recipient_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().to_str().unwrap()).unwrap();
        store.register(&Goal::new("g", "goal", ".")).unwrap();
        for (id, target) in [
            ("a1", Some("a")),
            ("b1", Some("b")),
            ("all", None),
            ("a2", Some("a")),
        ] {
            store
                .append(Event::ControlIssued {
                    goal_id: "g".into(),
                    instruction: Instruction {
                        id: id.into(),
                        agent_id: target.map(str::to_owned),
                        text: id.into(),
                        interrupt: true,
                    },
                    ts: 1,
                })
                .unwrap();
        }
        assert_eq!(
            pending(&store, "g", Some("a"))
                .unwrap()
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            ["all", "a2"]
        );
        store
            .append(Event::ControlAcknowledged {
                goal_id: "g".into(),
                instruction_id: "all".into(),
                agent_id: Some("a".into()),
                ts: 2,
            })
            .unwrap();
        drop(store);
        let store = Store::open(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(pending(&store, "g", Some("a")).unwrap()[0].id, "a2");
        assert_eq!(
            pending(&store, "g", Some("b"))
                .unwrap()
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            ["b1", "all"]
        );
    }
}
