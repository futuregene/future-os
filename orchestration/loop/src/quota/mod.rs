//! Quota subdomain (G-7) — slot accounting, usage summaries, and the stall
//! repair delivery guard.
//!
//! LoopX `control_plane/quota/` is 19 subdomains (6,685 lines); our P0 folded
//! quota into the `decide_for` function (a `QUOTA_ALLOWED_SLOTS` constant
//! plus `spent = history.len()`). G-7 splits that into testable subdomains
//! without changing the packet output:
//!
//!   - [`slot_accounting`] spend-source classification (run/agent/heartbeat)
//!     + per-goal spend breakdown, owning the allowed-slots constant.
//!   - [`usage_summary`]   24h/7d usage totals + per-goal rows (LoopX
//!     `build_usage_summary`).
//!   - [`stall_repair`]    stall detection → replan hint (the delivery guard
//!     that generalizes the old `MAX_REPAIR_ATTEMPTS` shortcut).

pub mod slot_accounting;
pub mod stall_repair;
pub mod usage_summary;
