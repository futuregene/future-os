# future-rpc

Wire contract between FutureAgent and its clients (TUI / CLI / channel
bridge / GUI backend).

This crate is the **single owner** of the generated proto code for
`../proto/future.proto` (both tonic server and client modules) and of the
typed-RPC payload contract: the shared payload structs plus the encode/decode
layer. The per-crate generated copies that used to live in `agent/`,
`channels/` and `gui/src-tauri/` have been retired onto this crate; the
TypeScript clients share the same contract through the sibling npm package
`future-rpc/ts` (`@future-os/rpc`).

## Typed payloads + dual-write

`RpcResponse.payload` and `StreamEvent.payload` (both at field number 20)
carry typed `oneof` payloads for the Tier-1 commands/events. During the
migration window the agent **dual-writes**: the typed `payload` and the
legacy JSON `data` string are both populated. Decoders
(`decode::response_data` / `decode::event_data`) read the typed payload when
present and fall back to parsing `data`, so new clients work against old
agents and old clients keep reading `data` from new agents. Once every
released client reads the typed payload, the `data` dual-write can be
retired (a later milestone).

- `encode.rs` — JSON `data` Value → typed `payload` (agent side). Defensive:
  returns `None` on unknown/shape-mismatched input so clients fall back.
- `decode.rs` — typed `payload` → canonical Value (client side), JSON `data`
  fallback. `event_data_json` prefers the original `data` string while it is
  still dual-written (byte-stable for persistence / NATS republish).
- `payloads.rs` / `payloads_ext.rs` / `event_payloads.rs` — the serde payload
  carriers shared by encode and decode (parity by construction).
- `events.rs` — `AgentEvent` enum + `parse_agent_event` (channel-bridge view).

## Proto codegen

Regeneration is opt-in and gated behind the `REGENERATE_PROTO` env var so
normal builds never need `protoc`:

```sh
REGENERATE_PROTO=1 cargo build -p future-rpc   # or: make generate-proto
```

The generated output (`src/generated/proto.rs`) is checked into git. CI has a
freshness gate that regenerates and fails on any diff.

## Contract rules

- Proto field numbers are stable and MUST NOT be reused (see the header of
  `../proto/future.proto`). Typed payload `oneof` members are append-only.
- Typed payloads attach to their host messages (`RpcResponse`, `StreamEvent`,
  `ProjectedRunEvent`, `ReplayEvent`) at field number 20; the JSON `data`
  field stays dual-written during the migration window.
- proto3 fields whose JSON form distinguishes null/absent from a default are
  declared `optional` so the typed path preserves the JSON semantics.
- This crate depends only on `tonic`/`prost`/`serde`/`serde_json`. All
  consumers (`future-agent`, `future-channel`, the GUI Tauri backend via a
  path dependency) depend on it — never the other way around. The GUI backend
  lives in its own cargo workspace: keep its `tonic`/`prost` versions aligned
  with the root `workspace.dependencies` pins.
