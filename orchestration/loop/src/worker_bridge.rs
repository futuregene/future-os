//! Worker bridge — the LoopX custom-runner contract over stdio.
//!
//! A worker (external process/script) connects to the control plane: each
//! tick the bridge emits the typed packet as one JSON line on stdout, the
//! worker executes a bounded turn in its own runtime, then writes one JSON
//! line back with the result. The bridge folds it into the state ledger and
//! re-decides. This is the same contract as `loopx run` but with the
//! EXECUTOR owned by the caller (LoopX: worker bridge install contract).

use std::io::{BufRead, Write};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::contract::TurnMode;
use crate::decision::decide_for;
use crate::executor::writeback;
use crate::state::{now_epoch, RunRecord, TodoStatus};
use crate::store::{Event, Store};

/// One line the worker writes back after executing a bounded turn.
#[derive(Debug, Deserialize)]
pub struct WorkerResult {
    pub todo_id: String,
    #[serde(default)]
    pub terminal_state: String,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// One line the bridge emits: the typed packet + the selected todo.
#[derive(Debug, Serialize)]
pub struct WorkerTurn {
    pub decision: String,
    pub mode: String,
    pub todo_id: Option<String>,
    pub todo_text: Option<String>,
    pub reason: String,
    pub should_run: bool,
}

pub struct BridgeOptions {
    pub goal_id: String,
    pub agent_id: Option<String>,
    pub max_turns: u32,
}

/// Run the worker bridge loop: emit packet → read worker result → writeback.
pub async fn run_bridge(store: &mut Store, opts: &BridgeOptions) -> Result<()> {
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout();

    for turn in 1..=opts.max_turns {
        let goal = store
            .replay(&opts.goal_id)?
            .ok_or_else(|| anyhow::anyhow!("goal {} not found", opts.goal_id))?;
        let packet = decide_for(
            &goal,
            std::time::SystemTime::now(),
            opts.agent_id.as_deref(),
        );
        let mode = packet.interaction_contract.mode;
        if mode == TurnMode::Terminal {
            println!("BRIDGE terminal: validated closure — stopping");
            return Ok(());
        }
        if !packet.should_run {
            println!(
                "BRIDGE stop: decision={} reason={}",
                packet.decision, packet.reason
            );
            return Ok(());
        }
        let sel = packet
            .interaction_contract
            .agent_channel
            .selected_todo
            .clone();
        let worker_turn = WorkerTurn {
            decision: packet.decision.clone(),
            mode: mode.as_str().to_string(),
            todo_id: sel.clone(),
            todo_text: sel
                .as_deref()
                .and_then(|id| goal.todo(id))
                .map(|t| t.text.clone()),
            reason: packet.reason.clone(),
            should_run: true,
        };
        let line = serde_json::to_string(&worker_turn)?;
        println!("BRIDGE packet: {line}");
        stdout.flush()?;

        // Read the worker's result line.
        let mut raw = String::new();
        stdin.read_line(&mut raw)?;
        let raw = raw.trim();
        if raw.is_empty() || raw == "BRIDGE done" {
            println!("BRIDGE worker finished; closing.");
            return Ok(());
        }
        let result: WorkerResult = serde_json::from_str(raw)
            .map_err(|e| anyhow::anyhow!("invalid worker result line `{raw}`: {e}"))?;

        let mut goal = store
            .replay(&opts.goal_id)?
            .ok_or_else(|| anyhow::anyhow!("goal {} not found", opts.goal_id))?;
        let record = RunRecord {
            turn,
            todo_id: result.todo_id.clone(),
            run_id: format!("worker-{turn}-{}", crate::state::now_epoch()),
            terminal_state: if result.terminal_state.is_empty() {
                "completed".to_string()
            } else {
                result.terminal_state.clone()
            },
            error: result.error.clone(),
            tokens_in_delta: 0,
            tokens_out_delta: 0,
            cost_delta: 0.0,
            tools: result.tools.clone(),
            evidence: result.evidence.clone(),
            recorded_at: now_epoch(),
            spend_source: None,
            validation: None,
        };
        // Completion contract: last remaining todo closes with no-follow-up;
        // otherwise remaining todos become successors.
        let successors: Vec<String> = goal
            .runnable_advancement_for(opts.agent_id.as_deref())
            .filter(|t| t.id != result.todo_id)
            .map(|t| t.id.clone())
            .collect();
        let is_last = successors.is_empty();
        let completion = if record.terminal_state == "completed" {
            Some((is_last, successors.clone()))
        } else {
            None
        };
        writeback(&mut goal, &record, None, completion);
        store.append_run(&opts.goal_id, &record)?;
        store.append(Event::RunRecorded {
            goal_id: opts.goal_id.clone(),
            record: record.clone(),
            ts: now_epoch(),
        })?;
        if record.terminal_state == "completed" {
            store.append(Event::TodoCompleted {
                goal_id: opts.goal_id.clone(),
                todo_id: result.todo_id.clone(),
                no_follow_up: is_last,
                successor_ids: successors.clone(),
                evidence: Some(record.evidence.clone()),
                ts: now_epoch(),
            })?;
        }
        let next_text = goal
            .runnable_advancement_for(opts.agent_id.as_deref())
            .next()
            .map(|t| t.text.clone())
            .unwrap_or_else(|| "all todos complete; no further action".to_string());
        store.set_next_action(&opts.goal_id, &next_text)?;
        println!(
            "BRIDGE writeback: {} → {}",
            result.todo_id, record.terminal_state
        );
        let _ = turn;
    }
    bail!("max-turns reached");
}

#[allow(dead_code)]
fn _status_label(t: &crate::state::Todo) -> &'static str {
    if t.status == TodoStatus::Done {
        "done"
    } else {
        "open"
    }
}
