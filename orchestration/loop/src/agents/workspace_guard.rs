//! Agent workspace guard (P0-1) — LoopX
//! `control_plane/agents/workspace_guard.py`, natively. Multi-agent
//! shared-workspace write-conflict protection: registered agents declare
//! the workspace path set they write into; claiming a todo while a peer
//! holds a live lease in an overlapping workspace is a conflict — the
//! claim degrades to serial (refused with a retry hint) unless the caller
//! passes an explicit `--force`.
//!
//! Todo write scopes take precedence over the agent's fallback workspaces.
//! The guard is ADVISORY and fail-open only when neither declares a write
//! set. Every successful claim with a write set also
//! appends a `WorkspaceLockAcquired` ledger event so `agent list` can show
//! who occupies which paths.

use crate::state::Goal;

pub const WORKSPACE_GUARD_SCHEMA_VERSION: &str = "agent_workspace_guard_v1";

/// Expand a leading `~` to the user's home directory (HOME, else
/// USERPROFILE on Windows). Anything else is returned unchanged.
fn expand_home(raw: &str) -> String {
    expand_home_with(
        raw,
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_default(),
    )
}

/// Deterministic core of [`expand_home`]: the home value is passed in so the
/// empty-home fallback is testable without mutating process env.
fn expand_home_with(raw: &str, home: String) -> String {
    let tilde = raw == "~" || raw.starts_with("~/") || raw.starts_with("~\\");
    if tilde && !home.is_empty() {
        format!("{}{}", home, &raw[1..])
    } else {
        raw.to_string()
    }
}

/// Lexically normalize a path (resolve `.`/`..` components, drop trailing
/// separators) WITHOUT touching the filesystem — the fallback for
/// workspace paths that do not exist yet.
fn lexical_normalize(path: &std::path::Path) -> String {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // Keep leading `..` on relative paths; otherwise pop.
                if out.file_name().is_some_and(|name| name != "..") {
                    out.pop();
                } else if !out.has_root() {
                    out.push(comp.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

/// Normalize a declared workspace path for storage: expand `~`, absolutize
/// against the process cwd, canonicalize when the path exists (resolves
/// symlinks such as /tmp → /private/tmp on macOS), otherwise fall back to
/// lexical normalization. Storage stays a plain string so replay is
/// platform-agnostic.
pub fn normalize_workspace_path(raw: &str) -> String {
    normalize_workspace_path_at(
        raw,
        &std::env::current_dir().expect("invariant: process cwd must be readable"),
    )
}

/// Resolve against a stable project anchor, including symlinked parents of
/// files that have not been created yet (e.g. /tmp/new.md on macOS).
pub fn normalize_workspace_path_at(raw: &str, base: &std::path::Path) -> String {
    let path = std::path::PathBuf::from(expand_home(raw.trim()));
    let abs = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    // Resolve existing prefixes before interpreting later `..` components.
    // Lexically collapsing the whole path first would lose symlink semantics;
    // canonicalizing only the full path misses not-yet-created output files.
    let mut resolved = std::path::PathBuf::new();
    for component in abs.components() {
        resolved.push(component.as_os_str());
        resolved = std::path::PathBuf::from(lexical_normalize(&resolved));
        if let Ok(canon) = resolved.canonicalize() {
            resolved = canon;
        }
    }
    resolved.to_string_lossy().into_owned()
}

/// True when two workspace paths overlap: equal, or one is an ancestor of
/// the other. Component-aware, so `/repo/a` never overlaps `/repo/ab`
/// while `/repo/a` and `/repo/a/sub` do.
pub fn paths_overlap(a: &str, b: &str) -> bool {
    // Windows paths are case-insensitive; preserve component boundaries.
    #[cfg(windows)]
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let pa = std::path::Path::new(&a);
    let pb = std::path::Path::new(&b);
    pa == pb || pa.starts_with(pb) || pb.starts_with(pa)
}

/// One live workspace conflict: another registered agent holds a live
/// lease while its declared workspace set overlaps the claimer's.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceConflict {
    pub schema_version: String,
    /// The OTHER agent currently occupying the overlapping workspace.
    pub holder_agent_id: String,
    /// Todos the holder owns under a live lease right now.
    pub holder_todo_ids: Vec<String>,
    /// Holder workspace paths that overlap the claimer's set.
    pub overlapping_paths: Vec<String>,
    /// Earliest live-lease expiry of the holder — the serial retry hint
    /// ("rerun after this epoch, or pass --force").
    pub holder_lease_expires_at: u64,
}

/// The agent's fallback write set, used only when a task has no write scopes.
pub fn agent_workspaces(goal: &Goal, agent_id: &str) -> Vec<String> {
    goal.agent_profiles
        .iter()
        .find(|p| p.id == agent_id)
        .map(|p| p.workspaces.clone())
        .unwrap_or_default()
}

/// Agent-list advisory: compare this agent's active task scopes (or its
/// fallback declaration when idle) with peers' live task scopes. Actual claims
/// must use `todo_workspace_conflicts` with the selected task under the lock.
pub fn live_workspace_conflicts(goal: &Goal, agent_id: &str, now: u64) -> Vec<WorkspaceConflict> {
    let held: Vec<_> = goal
        .todos
        .iter()
        .filter(|t| live_holder(t, agent_id, now))
        .collect();
    let mine = if held.is_empty() {
        agent_workspaces(goal, agent_id)
    } else {
        held.iter()
            .flat_map(|t| todo_workspaces(goal, agent_id, t))
            .collect()
    };
    conflicts_for_paths(goal, agent_id, &mine, now)
}

/// Effective write set: task declaration, or the agent's conservative fallback.
/// Relative task paths are anchored to the goal, never the inspecting worker cwd.
pub fn todo_workspaces(goal: &Goal, agent_id: &str, todo: &crate::state::Todo) -> Vec<String> {
    let scopes: Vec<_> = todo
        .required_write_scope
        .iter()
        .filter(|s| !s.trim().is_empty())
        .collect();
    if scopes.is_empty() {
        return agent_workspaces(goal, agent_id);
    }
    scopes
        .iter()
        .map(|s| normalize_workspace_path_at(s, std::path::Path::new(&goal.cwd)))
        .collect()
}

pub fn todo_workspace_conflicts(
    goal: &Goal,
    agent_id: &str,
    todo: &crate::state::Todo,
    now: u64,
) -> Vec<WorkspaceConflict> {
    conflicts_for_paths(goal, agent_id, &todo_workspaces(goal, agent_id, todo), now)
}

fn live_holder(todo: &crate::state::Todo, agent_id: &str, now: u64) -> bool {
    !matches!(
        todo.status,
        crate::state::TodoStatus::Done | crate::state::TodoStatus::Superseded
    ) && todo.claimed_by.as_deref() == Some(agent_id)
        && todo.lease_expires_at.is_some_and(|expires| expires > now)
        && todo.holder_pid.is_none_or(crate::compat::pid_alive)
}

fn conflicts_for_paths(
    goal: &Goal,
    agent_id: &str,
    mine: &[String],
    now: u64,
) -> Vec<WorkspaceConflict> {
    if mine.is_empty() {
        return vec![];
    }
    let mut conflicts = vec![];
    let holders: std::collections::BTreeSet<_> = goal
        .todos
        .iter()
        .filter_map(|t| t.claimed_by.as_deref())
        .filter(|id| *id != agent_id)
        .collect();
    for holder in holders {
        let mut overlapping = Vec::new();
        let mut held: Vec<&crate::state::Todo> = goal
            .todos
            .iter()
            .filter(|t| {
                if !live_holder(t, holder, now) {
                    return false;
                }
                let paths: Vec<_> = todo_workspaces(goal, holder, t)
                    .into_iter()
                    .filter(|w| mine.iter().any(|m| paths_overlap(m, w)))
                    .collect();
                let overlaps = !paths.is_empty();
                overlapping.extend(paths);
                overlaps
            })
            .collect();
        overlapping.sort();
        overlapping.dedup();
        if overlapping.is_empty() {
            continue;
        }
        held.sort_by(|a, b| a.id.cmp(&b.id));
        let earliest = held
            .iter()
            .filter_map(|t| t.lease_expires_at)
            .min()
            .unwrap_or(now);
        conflicts.push(WorkspaceConflict {
            schema_version: WORKSPACE_GUARD_SCHEMA_VERSION.to_string(),
            holder_agent_id: holder.to_string(),
            holder_todo_ids: held.iter().map(|t| t.id.clone()).collect(),
            overlapping_paths: overlapping,
            holder_lease_expires_at: earliest,
        });
    }
    conflicts.sort_by(|a, b| a.holder_agent_id.cmp(&b.holder_agent_id));
    conflicts
}

/// Render conflicts as a human report for CLI refusal messages (the
/// serial-degradation hint: wait for the holder's lease, or force).
pub fn render_conflicts(conflicts: &[WorkspaceConflict], now: u64) -> String {
    let mut out = String::new();
    for c in conflicts {
        let wait = c.holder_lease_expires_at.saturating_sub(now);
        out.push_str(&format!(
            "  ⚠ agent `{}` is writing {} (todos: {}; lease expires in {}s)\n",
            c.holder_agent_id,
            c.overlapping_paths.join(", "),
            c.holder_todo_ids.join(", "),
            wait
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AgentProfile, Goal, Todo};

    fn goal_with(agents: &[(&str, Vec<&str>)], todos: Vec<Todo>) -> Goal {
        let mut goal = Goal::new("g1", "objective", "/tmp");
        for (id, ws) in agents {
            goal.registered_agents.push(id.to_string());
            goal.agent_profiles.push(AgentProfile {
                id: id.to_string(),
                capabilities: vec![],
                workspaces: ws.iter().map(|s| s.to_string()).collect(),
            });
        }
        goal.todos = todos;
        goal
    }

    fn claimed(todo_id: &str, holder: &str, expires_at: u64) -> Todo {
        let mut t = Todo::advancement(todo_id, "work");
        t.claimed_by = Some(holder.to_string());
        t.lease_expires_at = Some(expires_at);
        t
    }

    #[test]
    fn completed_or_dead_holder_does_not_reserve_workspace() {
        let now = crate::state::now_epoch();
        let workspace = std::env::temp_dir()
            .join("loop-workspace-regression")
            .to_string_lossy()
            .into_owned();
        let mut t = claimed("t", "a", now + 3600);
        t.holder_pid = Some(std::process::id());
        let mut goal = goal_with(&[("a", vec![&workspace]), ("b", vec![&workspace])], vec![t]);
        assert_eq!(live_workspace_conflicts(&goal, "b", now).len(), 1);
        goal.todos[0].complete(true, vec![]);
        assert!(live_workspace_conflicts(&goal, "b", now).is_empty());
        // Terminal filtering also protects legacy snapshots carrying stale leases.
        goal.todos[0].claimed_by = Some("a".into());
        goal.todos[0].lease_expires_at = Some(now + 3600);
        assert!(live_workspace_conflicts(&goal, "b", now).is_empty());
        #[cfg(unix)]
        {
            goal.todos[0].status = crate::state::TodoStatus::Open;
            goal.todos[0].holder_pid = Some(i32::MAX as u32);
            assert!(live_workspace_conflicts(&goal, "b", now).is_empty());
        }
    }

    // ── path normalization ───────────────────────────────────────────────

    #[test]
    fn normalize_absolutizes_relative_paths_against_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let got = normalize_workspace_path("sub/dir");
        let canon = cwd.canonicalize().unwrap_or(cwd.clone());
        assert_eq!(got, format!("{}/sub/dir", canon.to_string_lossy()));
    }

    #[test]
    fn normalize_resolves_dot_components_lexically() {
        // Path that cannot exist → lexical fallback resolves `a/./b` and `..`.
        let got = normalize_workspace_path("/definitely/not/here/./x/../y");
        assert_eq!(got, "/definitely/not/here/y");
    }

    #[test]
    fn lexical_normalize_handles_curdir_and_leading_parentdir() {
        assert_eq!(lexical_normalize(std::path::Path::new("./foo")), "foo");
        assert_eq!(lexical_normalize(std::path::Path::new("../foo")), "../foo");
        assert_eq!(lexical_normalize(std::path::Path::new("a/../b")), "b");
    }

    #[test]
    fn expand_home_handles_tilde_and_empty_home() {
        assert_eq!(expand_home_with("~/x", "/home/u".into()), "/home/u/x");
        assert_eq!(expand_home_with("~", "/home/u".into()), "/home/u");
        assert_eq!(expand_home_with("~/x", String::new()), "~/x");
        assert_eq!(expand_home_with("~\\x", "C:\\u".into()), "C:\\u\\x");
        assert_eq!(expand_home_with("plain", "/home/u".into()), "plain");
    }

    #[test]
    fn normalize_expands_home_tilde() {
        // On the measurement hosts (macOS / Linux CI) HOME is always set; the
        // empty-home fallback is covered deterministically by
        // `expand_home_handles_tilde_and_empty_home`.
        let got = normalize_workspace_path("~/some-workspace");
        assert!(!got.contains('~'), "tilde must expand: {got}");
        assert!(got.ends_with("/some-workspace"), "got: {got}");
    }

    // ── overlap semantics ────────────────────────────────────────────────

    #[test]
    fn overlap_is_component_boundary_aware() {
        assert!(paths_overlap("/repo/wt1", "/repo/wt1"));
        assert!(paths_overlap("/repo/wt1", "/repo/wt1/src"));
        assert!(paths_overlap("/repo/wt1/src", "/repo/wt1"));
        assert!(!paths_overlap("/repo/wt1", "/repo/wt12"));
        assert!(!paths_overlap("/repo/wt1", "/repo/wt2"));
    }

    // ── conflict computation ─────────────────────────────────────────────

    #[test]
    fn conflict_when_peer_holds_live_lease_in_overlapping_workspace() {
        let goal = goal_with(
            &[
                ("agent-a", vec!["/repo/wt1"]),
                ("agent-b", vec!["/repo/wt1"]),
            ],
            vec![claimed("t1", "agent-b", 2000)],
        );
        let conflicts = live_workspace_conflicts(&goal, "agent-a", 1000);
        assert_eq!(conflicts.len(), 1);
        let c = &conflicts[0];
        assert_eq!(c.holder_agent_id, "agent-b");
        assert_eq!(c.holder_todo_ids, vec!["t1"]);
        assert_eq!(c.overlapping_paths, vec!["/repo/wt1"]);
        assert_eq!(c.holder_lease_expires_at, 2000);
        assert_eq!(c.schema_version, WORKSPACE_GUARD_SCHEMA_VERSION);
    }

    #[test]
    fn no_conflict_when_peer_lease_expired_or_peer_idle() {
        // Expired lease → free again.
        let goal = goal_with(
            &[
                ("agent-a", vec!["/repo/wt1"]),
                ("agent-b", vec!["/repo/wt1"]),
            ],
            vec![claimed("t1", "agent-b", 500)],
        );
        assert!(live_workspace_conflicts(&goal, "agent-a", 1000).is_empty());
        // Registered but holding nothing → no occupancy.
        let goal = goal_with(
            &[
                ("agent-a", vec!["/repo/wt1"]),
                ("agent-b", vec!["/repo/wt1"]),
            ],
            vec![],
        );
        assert!(live_workspace_conflicts(&goal, "agent-a", 1000).is_empty());
    }

    #[test]
    fn no_conflict_for_disjoint_workspaces_or_self_claim() {
        // Disjoint sets never conflict.
        let goal = goal_with(
            &[
                ("agent-a", vec!["/repo/wt1"]),
                ("agent-b", vec!["/repo/wt2"]),
            ],
            vec![claimed("t1", "agent-b", 2000)],
        );
        assert!(live_workspace_conflicts(&goal, "agent-a", 1000).is_empty());
        // The claimer's own live todos never conflict with itself.
        let goal = goal_with(
            &[("agent-a", vec!["/repo/wt1"])],
            vec![claimed("t1", "agent-a", 2000)],
        );
        assert!(live_workspace_conflicts(&goal, "agent-a", 1000).is_empty());
    }

    #[test]
    fn undeclared_workspace_is_fail_open() {
        // Claimer declares nothing → cannot assess → no conflict.
        let goal = goal_with(
            &[("agent-a", vec![]), ("agent-b", vec!["/repo/wt1"])],
            vec![claimed("t1", "agent-b", 2000)],
        );
        assert!(live_workspace_conflicts(&goal, "agent-a", 1000).is_empty());
        // Holder declares nothing → advisory guard stays silent.
        let goal = goal_with(
            &[("agent-a", vec!["/repo/wt1"]), ("agent-b", vec![])],
            vec![claimed("t1", "agent-b", 2000)],
        );
        assert!(live_workspace_conflicts(&goal, "agent-a", 1000).is_empty());
    }

    #[test]
    fn render_lists_holder_paths_and_serial_hint() {
        let goal = goal_with(
            &[
                ("agent-a", vec!["/repo/wt1"]),
                ("agent-b", vec!["/repo/wt1"]),
            ],
            vec![claimed("t1", "agent-b", 2000)],
        );
        let conflicts = live_workspace_conflicts(&goal, "agent-a", 1000);
        let text = render_conflicts(&conflicts, 1000);
        assert!(text.contains("agent-b"), "got: {text}");
        assert!(text.contains("/repo/wt1"), "got: {text}");
        assert!(text.contains("1000s"), "wait hint missing: {text}");
    }
}
