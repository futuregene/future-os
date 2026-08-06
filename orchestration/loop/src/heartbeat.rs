//! Heartbeat prompt — the per-turn re-entry packet for a host executor.
//!
//! LoopX's heartbeat contract: each wake compiles the current gate, next
//! todo, evidence refs, and stop conditions into a compact prompt the host
//! hands to the agent. The agent never "remembers" a previous packet — it
//! reads a fresh one every turn (LoopX: CLI packet is the per-turn process
//! contract, rebuilt from canonical state).

use crate::contract::ShouldRunPacket;
use crate::state::Goal;

/// Render a host-facing heartbeat prompt (markdown).
pub fn render_heartbeat_prompt(goal: &Goal, packet: &ShouldRunPacket) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# reference Heartbeat Packet — {}\n\n",
        goal.goal_id
    ));
    out.push_str(&format!("- objective: {}\n", goal.objective));
    out.push_str(&format!(
        "- decision: `{}` / mode: `{}`\n",
        packet.decision,
        packet.interaction_contract.mode.as_str()
    ));
    out.push_str(&format!("- should_run: `{}`\n", packet.should_run));
    out.push_str(&format!("- reason: {}\n", packet.reason));

    // User channel.
    let uc = &packet.interaction_contract.user_channel;
    if uc.action_required {
        out.push_str(&format!(
            "- USER ACTION REQUIRED: {}\n",
            uc.question.as_deref().unwrap_or("(see todo)")
        ));
        out.push_str(&format!("- gate todos: {}\n", uc.todo_ids.join(", ")));
    } else {
        out.push_str("- user channel: none\n");
    }

    // Agent channel.
    let ac = &packet.interaction_contract.agent_channel;
    if let Some(sel) = &ac.selected_todo {
        let todo = goal.todo(sel);
        out.push_str(&format!("- NEXT TODO: {sel}\n"));
        if let Some(t) = todo {
            out.push_str(&format!("  - text: {}\n", t.text));
            if t.failed_attempts > 0 {
                out.push_str(&format!("  - repair attempt {}\n", t.failed_attempts + 1));
            }
        }
    }
    if let Some(fb) = &ac.fallback_todo {
        out.push_str(&format!("- fallback todo (gate-independent): {fb}\n"));
    }

    // Evidence + next action.
    if let Some(next) = &goal.next_action {
        out.push_str(&format!("- next action: {next}\n"));
    }
    let last = goal.history.last();
    if let Some(r) = last {
        out.push_str(&format!(
            "- last evidence: {} (todos {})\n",
            crate::decision::truncate(&r.evidence, 200),
            r.tools.join(",")
        ));
    }

    // Spend / scheduler / boundary.
    out.push_str(&format!(
        "- quota: {} slots spent / {}\n",
        packet.quota.spent_slots, packet.quota.allowed_slots
    ));
    out.push_str(&format!(
        "- scheduler: {} ({})\n",
        packet.scheduler_hint.action, packet.scheduler_hint.cadence_class
    ));
    if packet.boundary.public_safe {
        out.push_str("- boundary: public-safe ✔\n");
    } else {
        out.push_str(&format!(
            "- ⚠ boundary leaks: {}\n",
            packet.boundary.leaks.join("; ")
        ));
    }

    // Rules + next CLI actions.
    out.push_str("\n## Rules\n");
    out.push_str("- Work exactly the NEXT TODO. Do not invent work outside the goal.\n");
    out.push_str("- Write evidence: concrete results (paths, values, diffs).\n");
    out.push_str("- On completion declare a successor or no-follow-up.\n");
    out.push_str(&format!(
        "- stop condition: {}\n",
        match packet.interaction_contract.mode {
            crate::contract::TurnMode::Terminal => "goal validated closed — stop automation",
            _ => "continue the bounded segment; the next heartbeat re-decides",
        }
    ));
    out.push_str("\n## Next CLI actions (after the turn)\n");
    out.push_str("- `loopx refresh-state --goal G --next-action <frontier>`\n");
    out.push_str("- `loopx quota spend-slot --goal G` (validated work only)\n");
    out
}
