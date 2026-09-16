# Decision-kernel regression baseline (P0 snapshot)

Byte-identical snapshot of the pre-split `src/decision.rs` (696 lines). It is
the frozen baseline for the G-1 kernel modularization: `decision.rs` was split
into `src/decision/` subdomain modules (identity / boundary / frontier /
monitor / stall / heartbeat recommendation / goal boundary / primary action),
and this snapshot anchors the field-for-field packet-parity regression.

## Snapshot files

| File | Description |
|---|---|
| `decision.rs.pre-split` | Byte-identical copy of the pre-split `src/decision.rs` (696 lines), the full pre-split decision kernel |

## Baseline metadata

- **Source commit**: `85372a53a0b61f57ba492894788249fb66315b94`
  (branch `claude/loop-orchestrator`, "full lifecycle process + quota packet
  parity", 2026-08-05)
- **SHA-256**: `4ac8c78e7e3f489e1304e57ce0e9f5dbc3bebc965b013913b487b89b9bebf165`
- **Lines**: 696
- **Scope**: `decide`/`decide_for` (should-run decision compilation), the
  `complete_todo` obligation (successor / no-follow-up), monitor due
  poll/backoff, replan (success/failure/stalled/acceptance gap), and
  `packet()` assembly (~40-field `ShouldRunPacket` + interaction contract
  channels + sub-contracts).
- **Baseline tests**: 76 contract tests across 11 files, green at snapshot
  time (2026-08-06).

## How the regression is enforced

The snapshot is consumed by `tests/decision_split_regression.rs`:

1. **Split regression (authoritative)**: the pre-split kernel is compiled
   verbatim as a test-only legacy module (`tests/legacy/decision_pre_split.rs`,
   mechanical transforms only) and run side-by-side with the refactored kernel
   over 20 fixtures covering every decision path; the serialized packets are
   compared field-for-field (recursive JSON, wall-clock/UUID masked). The only
   allowed delta is the G-2/G-11 scheduler-arbitration record.
2. **Provenance guard**: `generated_legacy_module_is_derived_from_snapshot`
   re-derives the legacy module from `decision.rs.pre-split` and verifies no
   snapshot line is lost — the snapshot cannot silently drift.
3. **Hash check** (out-of-band):
   ```sh
   shasum -a 256 orchestration/loop/snapshots/decision.rs.pre-split
   # expect 4ac8c78e7e3f489e1304e57ce0e9f5dbc3bebc965b013913b487b89b9bebf165
   ```

## Update policy

- The snapshot is **frozen**: only overwrite it when the split regression is
  green and the packet field/enum surface matches the snapshot semantics;
  then update this README's source-commit and hash fields.
- The persistent regression anchor is `tests/decision_split_regression.rs`
  (23 tests); the snapshot itself exists to keep the pre-split logic
  verifiable even after the source file is gone.
