//! Agent workspace guard (P0-1) — LoopX
//! `control_plane/agents/workspace_guard.py`, natively. Multi-agent
//! shared-workspace write-conflict protection: registered agents declare
//! the workspace path set they write into; claiming a todo while a peer
//! holds a live lease in an overlapping workspace is a conflict — the
//! claim degrades to serial (refused with a retry hint) unless the caller
//! passes an explicit `--force`.
//!
//! The guard is ADVISORY and fail-open: an agent that declares no
//! workspaces cannot be assessed and never blocks (legacy peers keep
//! working). Every successful claim by a workspace-declaring agent also
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
                if !out.pop() {
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
    let expanded = expand_home(raw.trim());
    let path = std::path::PathBuf::from(expanded);
    let abs = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .expect("invariant: process cwd must be readable")
            .join(path)
    };
    if let Ok(canon) = abs.canonicalize() {
        return canon.to_string_lossy().into_owned();
    }
    lexical_normalize(&abs)
}

/// True when two workspace paths overlap: equal, or one is an ancestor of
/// the other. Component-aware, so `/repo/a` never overlaps `/repo/ab`
/// while `/repo/a` and `/repo/a/sub` do.
pub fn paths_overlap(a: &str, b: &str) -> bool {
    let pa = std::path::Path::new(a);
    let pb = std::path::Path::new(b);
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

/// The declared workspace set of an agent (empty = undeclared → guard is
/// fail-open for that agent).
pub fn agent_workspaces(goal: &Goal, agent_id: &str) -> Vec<String> {
    goal.agent_profiles
        .iter()
        .find(|p| p.id == agent_id)
        .map(|p| p.workspaces.clone())
        .unwrap_or_default()
}

/// Compute the live workspace conflicts for `agent_id` claiming work at
/// `now`: every OTHER registered agent that (a) declares workspaces,
/// (b) holds at least one live lease, and (c) whose declared set overlaps
/// the claimer's. Empty = safe to claim. Fail-open: an empty claimer set
/// yields no conflicts (nothing to compare against).
pub fn live_workspace_conflicts(goal: &Goal, agent_id: &str, now: u64) -> Vec<WorkspaceConflict> {
    let mine = agent_workspaces(goal, agent_id);
    if mine.is_empty() {
        return vec![];
    }
    let mut conflicts = vec![];
    for profile in &goal.agent_profiles {
        if profile.id == agent_id || profile.workspaces.is_empty() {
            continue;
        }
        let mut held: Vec<&crate::state::Todo> = goal
            .todos
            .iter()
            .filter(|t| {
                t.claimed_by.as_deref() == Some(profile.id.as_str())
                    && t.lease_expires_at.map(|e| e > now).unwrap_or(false)
            })
            .collect();
        if held.is_empty() {
            continue;
        }
        let overlapping: Vec<String> = profile
            .workspaces
            .iter()
            .filter(|w| mine.iter().any(|m| paths_overlap(m, w)))
            .cloned()
            .collect();
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
            holder_agent_id: profile.id.clone(),
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
