//! Boundary snapshot — leak scan of the goal objective against the write
//! scope (LoopX: `boundary_scan_leaks`).

use crate::contract::BoundarySnapshot;
use crate::state::Goal;

/// Compose the boundary snapshot: the objective's leak scan plus the
/// public-safety flag derived from it.
pub(crate) fn boundary_snapshot(goal: &Goal) -> BoundarySnapshot {
    let leaks = crate::state::boundary_scan_leaks(&goal.objective);
    BoundarySnapshot {
        leaks,
        public_safe: crate::state::boundary_scan_leaks(&goal.objective).is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Goal;

    #[test]
    fn clean_objective_is_public_safe() {
        let g = Goal::new("g", "run the benchmark and record results", "/tmp");
        let bs = boundary_snapshot(&g);
        assert!(bs.public_safe);
        assert!(bs.leaks.is_empty());
    }

    #[test]
    fn sensitive_markers_are_leaks() {
        for marker in [".ssh", "token=", "api_key", "auth.json"] {
            let g = Goal::new(
                "g",
                &format!("copy credentials {marker} into the report"),
                "/tmp",
            );
            let bs = boundary_snapshot(&g);
            assert!(!bs.public_safe, "marker {marker} must be flagged");
            assert!(
                !bs.leaks.is_empty(),
                "marker {marker} must produce a leak record"
            );
        }
    }

    #[test]
    fn absolute_home_path_is_a_leak() {
        let home = std::env::var("HOME").expect("HOME is set in every test environment");
        let g = Goal::new("g", &format!("read {home}/secrets.txt"), "/tmp");
        assert!(!boundary_snapshot(&g).public_safe);
    }
}
