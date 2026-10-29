# Shared packages

The `packages/` directory contains reusable packages consumed by more than one
FutureOS application or service. Package names and public APIs are independent
of their implementation language.

- `rpc`: Rust wire-contract crate and protobuf source of truth.
- `remote-crypto`: Rust/TypeScript Noise secure channels sharing handshake, record and rejection fixtures.
- `remote-client`: Rust controller connections, credential DTOs, multi-host instances, task lifecycle and file orchestration; TS controller connections, host services, generic synchronization/projection, file orchestration and stop/drain are connected to Mobile; runtime/application acceptance awaits final CI.
- `remote-protocol`: Rust/TypeScript wire contracts, command schema/generated types, handshake envelopes, invitations and JSON/gzip codecs.
- `remote-server`: Rust controlled-endpoint security, host contracts, file framing, ordered events/coalescing, command admission, wire policies, NATS connections, heartbeat/catalog publishing and instance supervisor/control state; startup/recovery, credential rotation and role stop/drain are connected; runtime and interoperability acceptance still await final CI.
- `app-settings`: the desktop app's own settings document
  (`~/.future/app/app.db`), shared by the Tauri backend and `future desktop`.
- `app-workspaces`: the desktop app's workspace records in the same database,
  shared by the Tauri backend and `future workspace`.
- `config`: strict-read and atomic-write helpers for the hand-editable JSON
  files under `~/.future/` (`models.json`, `auth.json`, the TUI's
  `settings.json`), shared by the application host and the TUI.
- `http`: the HTTP client entry points every Rust consumer starts from, so the
  process TLS provider is installed once instead of per caller.
- `markdown`: Shared TypeScript markdown parser and types.
- `thread-projection`: Shared TypeScript thread projection logic.
- `json-preview`: Shared TypeScript JSON preview/rendering logic.

A package should have its own manifest, public entry point, and tests. Packages
may depend on other packages, but must not depend on product implementations
such as `desktop`, `mobile`, or `agent`.

The [Remote package development plan](remote-package-development.md) defines
the ongoing Rust/TypeScript protocol, client and server extraction, retaining
independent Crypto and application host adapters. Rust Client/Server and TS Client are connected to their applications. Independent dual-language interoperability and shared behavior drivers are established; final runtime, performance and manual acceptance remain pending.
