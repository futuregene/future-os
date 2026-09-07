# future-rpc

Wire contract between FutureAgent and its clients (TUI / CLI / channel
bridge / desktop backend).

This crate is the **single owner** of the generated proto code for
`proto/future.proto` (both tonic server and client modules) and of the
typed-RPC payload contract: the shared payload structs plus the encode/decode
layer. The per-crate generated copies that used to live in `agent/`,
`channels/` and `desktop/src-tauri/` have been retired onto this crate. Every
consumer is Rust; the former sibling npm package `future-rpc/ts`
(`@future-os/rpc`) was removed when the TUI/CLI were ported to Rust.

## Typed responses and dual-written events

`RpcResponse.payload` and `StreamEvent.payload` (both at field number 20)
carry typed `oneof` payloads for the Tier-1 commands/events.

- **Command-response dual-write is retired.** Typed commands carry `payload`
  with empty `data`; untyped commands keep JSON `data`. `decode::response_data`
  is typed-first with a JSON fallback. Old data-only clients cannot consume
  typed-only responses; keep clients and agent compatible.
- **Events still dual-write.** `data` remains byte-stable for journals/NATS.
  `decode::event_data` and `event_data_json` prefer existing `data`, with typed
  reconstruction as fallback. Do not apply response-only retirement to events.
- Payload JSON uses canonical camelCase; removed legacy alias injection/stripping
  is not part of the current contract.

The model/agent runtime no longer uses this string envelope internally: model
providers emit typed model events and the Agent projects typed run events onto
`StreamEvent` only at the RPC boundary. The wire envelope itself is still a
migration boundary, not the final design. Retiring it requires all sideband and
control-plane events to gain typed payload variants, a versioned migration for
journal/replay/NATS records, and an explicit compatibility window for released
clients. Until those prerequisites are complete, keep `type`/`data` dual-write
and replay semantics byte-compatible.

- `encode.rs` — JSON `data` Value → typed `payload` (agent side). Defensive:
  returns `None` on unknown/shape-mismatched input so clients fall back.
- `decode.rs` — response typed-first decoding and event data-first decoding,
  reconstructing canonical Values when needed. `event_data_json` prefers the original `data` string while it is
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
  `proto/future.proto`). Typed payload `oneof` members are append-only.
- Typed payloads attach to their host messages (`RpcResponse`, `StreamEvent`,
  `ProjectedRunEvent`, `ReplayEvent`) at field number 20; the JSON `data`
  field stays dual-written for events; typed command responses leave it empty.
- proto3 fields whose JSON form distinguishes null/absent from a default are
  declared `optional` so the typed path preserves the JSON semantics.
- `transport.rs` owns shared per-user IPC discovery and explicit TCP fallback;
  its dependencies include async/network and platform IPC support (see Cargo.toml).
  All
  consumers (`future-agent`, `future-channel`, the desktop Tauri backend via a
  path dependency) depend on it — never the other way around. The desktop backend
  lives in its own cargo workspace: keep its `tonic`/`prost` versions aligned
  with the root `workspace.dependencies` pins.
