//! Task lease state machine (G-13) — claim / renew / expiry / steal over
//! the event base (TodoClaimed + the new TodoRenewed / TodoReleased /
//! TodoExpired), mirroring LoopX `control_plane/work_items/task_lease.py`
//! (769 lines) in minimal form.
//!
//! A lease is the bounded execution window over a slice; claim is NOT
//! ownership (LoopX: "claim is not ownership; lease is the bounded execution
//! window"). An expired lease returns the todo to the frontier; a new agent
//! may (re)claim it — the steal is modeled as `TodoExpired` followed by a
//! fresh `TodoClaimed`, so replay is exact and idempotent.

use anyhow::{bail, Result};

use crate::state::{Todo, TodoStatus};

pub const TASK_LEASE_SCHEMA_VERSION: &str = "task_lease_v0";
pub const DEFAULT_TASK_LEASE_TTL_SECONDS: u64 = 45 * 60;
pub const MAX_TASK_LEASE_TTL_SECONDS: u64 = 24 * 60 * 60;

/// The derived lease state of a todo at a point in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseStatus {
    /// No claim at all (never claimed, or released).
    Free,
    /// A live lease held by `owner` until `expires_at`.
    Active { owner: String, expires_at: u64 },
    /// A lease that lapsed without renewal — the slice returns to the
    /// frontier and a new agent may steal it.
    Expired { owner: String, expires_at: u64 },
}

/// Lease state at `now` (LoopX `lease_is_active`).
pub fn lease_status(todo: &Todo, now: u64) -> LeaseStatus {
    match (&todo.claimed_by, todo.lease_expires_at) {
        (Some(owner), Some(expires)) if expires > now => LeaseStatus::Active {
            owner: owner.clone(),
            expires_at: expires,
        },
        (Some(owner), Some(expires)) => LeaseStatus::Expired {
            owner: owner.clone(),
            expires_at: expires,
        },
        _ => LeaseStatus::Free,
    }
}

/// Whether the todo currently holds a live lease (LoopX `lease_is_active`).
pub fn lease_is_active(todo: &Todo, now: u64) -> bool {
    matches!(lease_status(todo, now), LeaseStatus::Active { .. })
}

/// Normalize a TTL (LoopX `normalize_ttl_seconds`): default 45 min, max 24h.
pub fn normalize_ttl(ttl_seconds: u64) -> Result<u64> {
    let ttl = if ttl_seconds == 0 {
        DEFAULT_TASK_LEASE_TTL_SECONDS
    } else {
        ttl_seconds
    };
    if ttl > MAX_TASK_LEASE_TTL_SECONDS {
        bail!("ttl seconds must be between 1 and {MAX_TASK_LEASE_TTL_SECONDS}");
    }
    Ok(ttl)
}

/// The outcome of a successful claim (`steal` is true when the previous
/// lease had expired).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimOutcome {
    pub idempotent: bool,
    pub steal: bool,
}

/// The outcome of a lease operation (what the caller should persist).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseOp {
    /// Renew succeeded.
    Renewed,
    /// Release succeeded (`missing` = there was nothing to release).
    Released { missing: bool },
    /// Expiry recorded (`had_lease` = an expired claim was cleared).
    Expired { had_lease: bool },
}

/// Claim a slice (LoopX `acquire_task_lease`, minimal): the todo must be
/// open; a live lease held by ANOTHER agent conflicts; the same owner
/// re-claiming is idempotent; a free or expired lease is acquired (steal
/// after expiry).
pub fn claim(todo: &mut Todo, agent: &str, lease_secs: u64, now: u64) -> Result<ClaimOutcome> {
    if todo.status != TodoStatus::Open {
        bail!("task lease requires an open todo");
    }
    let ttl = normalize_ttl(lease_secs)?;
    match lease_status(todo, now) {
        LeaseStatus::Active { owner, .. } if owner == agent => Ok(ClaimOutcome {
            idempotent: true,
            steal: false,
        }),
        LeaseStatus::Active { .. } => {
            // Lease liveness: a dead holder's claim is reclaimed
            // automatically — kill -0 probe on the recorded holder pid
            // (missing pid = pre-liveness ledger, keep the old hard error).
            if let Some(pid) = todo.holder_pid {
                if !crate::compat::pid_alive(pid) {
                    todo.claimed_by = Some(agent.to_string());
                    todo.lease_expires_at = Some(now + ttl);
                    todo.holder_pid = Some(std::process::id());
                    todo.updated_at = now;
                    return Ok(ClaimOutcome {
                        idempotent: false,
                        steal: true,
                    });
                }
            }
            bail!("todo already has an active lease held by another agent")
        }
        LeaseStatus::Free => {
            todo.claimed_by = Some(agent.to_string());
            todo.lease_expires_at = Some(now + ttl);
            todo.holder_pid = Some(std::process::id());
            todo.updated_at = now;
            Ok(ClaimOutcome {
                idempotent: false,
                steal: false,
            })
        }
        LeaseStatus::Expired { .. } => {
            // Steal: the previous lease lapsed; clear it and re-claim.
            todo.claimed_by = Some(agent.to_string());
            todo.lease_expires_at = Some(now + ttl);
            todo.holder_pid = Some(std::process::id());
            todo.updated_at = now;
            Ok(ClaimOutcome {
                idempotent: false,
                steal: true,
            })
        }
    }
}

/// Renew a live lease owned by `agent` (LoopX `renew_task_lease`, minimal):
/// extends `lease_expires_at`; requires an active lease owned by the caller.
pub fn renew(todo: &mut Todo, agent: &str, lease_secs: u64, now: u64) -> Result<LeaseOp> {
    let ttl = normalize_ttl(lease_secs)?;
    match lease_status(todo, now) {
        LeaseStatus::Active { owner, .. } if owner == agent => {
            todo.lease_expires_at = Some(now + ttl);
            todo.updated_at = now;
            Ok(LeaseOp::Renewed)
        }
        LeaseStatus::Active { .. } => bail!("lease owner mismatch"),
        _ => bail!("lease is missing or expired"),
    }
}

/// Release the lease early (LoopX `release_task_lease`, minimal): only the
/// current owner may release; a free todo is a no-op (`missing`).
pub fn release(todo: &mut Todo, agent: &str, now: u64) -> Result<LeaseOp> {
    match lease_status(todo, now) {
        LeaseStatus::Active { owner, .. } if owner == agent => {
            todo.claimed_by = None;
            todo.lease_expires_at = None;
            todo.updated_at = now;
            Ok(LeaseOp::Released { missing: false })
        }
        LeaseStatus::Active { .. } => bail!("lease owner mismatch"),
        LeaseStatus::Free => Ok(LeaseOp::Released { missing: true }),
        // An expired lease is effectively free — releasing is a no-op.
        LeaseStatus::Expired { .. } => Ok(LeaseOp::Released { missing: true }),
    }
}

/// Record expiry explicitly (LoopX task-lease lifecycle): clears the lapsed
/// claim so the slice returns to the frontier (the steal path emits
/// TodoExpired before a fresh TodoClaimed).
pub fn expire(todo: &mut Todo, now: u64) -> Result<LeaseOp> {
    match lease_status(todo, now) {
        LeaseStatus::Expired { .. } => {
            todo.claimed_by = None;
            todo.lease_expires_at = None;
            todo.updated_at = now;
            Ok(LeaseOp::Expired { had_lease: true })
        }
        LeaseStatus::Free => Ok(LeaseOp::Expired { had_lease: false }),
        LeaseStatus::Active { .. } => bail!("lease is still active"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo_open() -> Todo {
        Todo::advancement("t1", "work")
    }

    #[test]
    fn claim_acquires_free_todo() {
        let mut todo = todo_open();
        let op = claim(&mut todo, "alice", 60, 1_000).unwrap();
        assert_eq!(
            op,
            ClaimOutcome {
                idempotent: false,
                steal: false
            }
        );
        assert_eq!(todo.claimed_by.as_deref(), Some("alice"));
        assert_eq!(todo.lease_expires_at, Some(1_060));
    }

    #[test]
    fn release_by_non_owner_is_rejected() {
        let mut todo = todo_open();
        claim(&mut todo, "alice", 60, 1_000).unwrap();
        let err = release(&mut todo, "bob", 1_010).unwrap_err();
        assert!(format!("{err:#}").contains("owner mismatch"), "{err:#}");
        // The lease is untouched.
        assert_eq!(todo.claimed_by.as_deref(), Some("alice"));
    }

    #[test]
    fn same_owner_reclaim_is_idempotent() {
        let mut todo = todo_open();
        claim(&mut todo, "alice", 60, 1_000).unwrap();
        let op = claim(&mut todo, "alice", 60, 1_010).unwrap();
        assert_eq!(
            op,
            ClaimOutcome {
                idempotent: true,
                steal: false
            }
        );
    }

    #[test]
    fn live_lease_conflicts_with_other_owner() {
        let mut todo = todo_open();
        claim(&mut todo, "alice", 60, 1_000).unwrap();
        assert!(claim(&mut todo, "bob", 60, 1_010).is_err());
    }

    #[test]
    fn expired_lease_allows_steal() {
        let mut todo = todo_open();
        claim(&mut todo, "alice", 60, 1_000).unwrap(); // expires at 1060
        let op = claim(&mut todo, "bob", 60, 2_000).unwrap(); // after expiry
        assert_eq!(
            op,
            ClaimOutcome {
                idempotent: false,
                steal: true
            }
        );
        assert_eq!(todo.claimed_by.as_deref(), Some("bob"));
        assert_eq!(todo.lease_expires_at, Some(2_060));
    }

    #[test]
    fn renew_extends_only_for_owner() {
        let mut todo = todo_open();
        claim(&mut todo, "alice", 60, 1_000).unwrap();
        assert!(renew(&mut todo, "bob", 60, 1_010).is_err());
        let op = renew(&mut todo, "alice", 60, 1_010).unwrap();
        assert_eq!(op, LeaseOp::Renewed);
        assert_eq!(todo.lease_expires_at, Some(1_070));
        // Renew after expiry fails (lease is missing or expired).
        let mut todo2 = todo_open();
        claim(&mut todo2, "alice", 60, 1_000).unwrap();
        assert!(renew(&mut todo2, "alice", 60, 2_000).is_err());
    }

    #[test]
    fn release_clears_claim() {
        let mut todo = todo_open();
        claim(&mut todo, "alice", 60, 1_000).unwrap();
        let op = release(&mut todo, "alice", 1_010).unwrap();
        assert_eq!(op, LeaseOp::Released { missing: false });
        assert_eq!(todo.claimed_by, None);
        // Releasing a free todo is a no-op.
        let op2 = release(&mut todo, "alice", 1_020).unwrap();
        assert_eq!(op2, LeaseOp::Released { missing: true });
    }

    #[test]
    fn expire_records_lapsed_lease() {
        let mut todo = todo_open();
        claim(&mut todo, "alice", 60, 1_000).unwrap();
        assert!(expire(&mut todo, 1_010).is_err(), "still active");
        let op = expire(&mut todo, 2_000).unwrap();
        assert_eq!(op, LeaseOp::Expired { had_lease: true });
        assert_eq!(todo.claimed_by, None);
        // Expiring a free todo is a no-op.
        let op2 = expire(&mut todo, 2_010).unwrap();
        assert_eq!(op2, LeaseOp::Expired { had_lease: false });
    }

    #[test]
    fn ttl_normalization() {
        assert_eq!(normalize_ttl(0).unwrap(), DEFAULT_TASK_LEASE_TTL_SECONDS);
        assert!(normalize_ttl(MAX_TASK_LEASE_TTL_SECONDS + 1).is_err());
        assert_eq!(normalize_ttl(120).unwrap(), 120);
    }
}
