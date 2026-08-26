# Upstream: LoopX

`future-loop` (this directory) contains code translated, ported, and
structurally adapted from **LoopX**, a control plane for long-running AI
agent work.

- **Upstream repository:** <https://github.com/huangruiteng/loopx>
- **Upstream author:** Ruiteng Huang and LoopX contributors
- **Upstream license:** Apache License, Version 2.0 (see [`LICENSE`](LICENSE)
  in this directory). LoopX releases through v0.4.7 were distributed under
  the MIT License; v0.4.8 is the first Apache-2.0 release (see [`NOTICE`](NOTICE)).

## Base version

- The derivation tracks the LoopX **v0.4.x** line, through
  [`v0.4.8`](https://github.com/huangruiteng/loopx/releases/tag/v0.4.8)
  (commit `8c103dfecae0f4424ecb0b07bad7cbc5f0797d6d`), per the upstream
  maintainer's review of this implementation (2026-08).
- Initial import into FutureOS: 2026-08-06, commit `e2a8fb84` (PR #97).

## Scope of derived code

LoopX is written in Python; `future-loop` is a native Rust
re-implementation of its control plane. Derived subsystems include:

- the deterministic should-run decision kernel and its decision subdomains
  (identity / boundary / frontier / monitor / stall / heartbeat /
  goal-boundary / primary-action);
- the event-sourced state ledger, event replay, and markdown backfill;
- quota and slot accounting (run / agent / heartbeat);
- the scheduler arbitration layer and its dispositions;
- the markdown workbench and sidecar file formats
  (`ACTIVE_GOAL_STATE.md`, lockfiles, workbench layout);
- the `loopx`-style CLI command surface (console commands);
- agent registry / coordination, claim / lease, gates, monitors, and
  backup / restore.

Throughout the sources, comments marked `LoopX: ...` or
`` LoopX `<module>` `` map individual functions and behaviors to the
corresponding upstream modules.

## FutureGene modifications

FutureGene holds the copyright to its modifications and original
additions, including:

- the Rust-native implementation itself (type system, storage,
  concurrency, cross-platform support including Windows);
- the gRPC executor bridge to the FutureOS agent and the typed-RPC wire
  contract (`future-rpc` dual-written payloads);
- the unified `future loop` CLI and integration with the FutureOS TUI,
  desktop app, and skills;
- features with no upstream counterpart (canary smoke, automation
  liveness, read-model self-healing, pid lockfiles / zombie takeover);
- project-local state layout and FutureOS directory conventions.

## Relationship

Future Loop is an **independent downstream implementation** maintained by
FutureGene. It is **not** an official LoopX release and has **not** been
certified or endorsed by the LoopX project. Compatibility with upstream
LoopX state files is best-effort.
