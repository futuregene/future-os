//! work_items subdomain — task leases + autonomous replan obligation
//! bookkeeping (G-13), the task dependency graph (G-14), and the
//! attention / operator-inbox / delivery signal surfaces (G-15) that the P3
//! multi-agent work splitting builds on (LoopX `control_plane/work_items/`
//! 31 files; we cover the subset P2/P3 need).

pub mod attention;
pub mod delivery;
pub mod delivery_outcome;
pub mod operator_inbox;
pub mod replan_obligation;
pub mod review_queue;
pub mod reviewer;
pub mod task_graph;
pub mod task_lease;
