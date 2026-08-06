# future-rpc

Wire contract between FutureAgent and its clients (TUI / CLI / channel
bridge / GUI backend).

This crate is the **single owner** of the generated proto code for
`proto/future.proto` (both tonic server and client modules). The per-crate
generated copies that used to live in `agent/`, `channels/` and
`gui/src-tauri/` are being retired onto this crate as part of the typed-RPC
milestone; subsequent batches also add the shared payload structs and the
encode/decode layer (typed `payload` oneof first, JSON `data` fallback).

## Proto codegen

Regeneration is opt-in and gated behind the `REGENERATE_PROTO` env var so
normal builds never need `protoc`:

```sh
REGENERATE_PROTO=1 cargo build -p future-rpc   # or: make generate-proto
```

The generated output (`src/generated/proto.rs`) is checked into git.

## Contract rules

- Proto field numbers are stable and MUST NOT be reused (see the header of
  `proto/future.proto`).
- Typed payloads attach to their host messages (`RpcResponse`,
  `StreamEvent`, `ProjectedRunEvent`, `ReplayEvent`) at field number 20;
  the JSON `data` field stays dual-written during the migration window.
- This crate depends only on `tonic`/`prost`/`serde`/`serde_json`. All
  consumers (`future-agent`, `future-channel`, the GUI Tauri backend via a
  path dependency) depend on it — never the other way around. The GUI
  backend lives in its own cargo workspace: keep its `tonic`/`prost`
  versions aligned with the root `workspace.dependencies` pins.
