# Shared packages

The `packages/` directory contains reusable packages consumed by more than one
FutureOS application or service. Package names and public APIs are independent
of their implementation language.

- `rpc`: Rust wire-contract crate and protobuf source of truth.
- `remote-crypto`: Rust/TypeScript Noise secure channels sharing handshake, record and rejection fixtures.
- `remote-client`: Rust controller connections, credential DTOs, multi-host instances, task lifecycle and file orchestration; TS Client remains in migration.
- `remote-protocol`: Rust/TypeScript wire contracts, command schema/generated types, handshake envelopes, invitations and JSON/gzip codecs.
- `remote-server`: Rust controlled-endpoint security, credential/status DTOs and business/state contracts; supervision and command routing remain in migration.
- `app-settings`: the desktop app's own settings document
  (`~/.future/app/app.db`), shared by the Tauri backend and `future desktop`.
- `app-workspaces`: the desktop app's workspace records in the same database,
  shared by the Tauri backend and `future workspace`.
- `markdown`: Shared TypeScript markdown parser and types.
- `thread-projection`: Shared TypeScript thread projection logic.
- `json-preview`: Shared TypeScript JSON preview/rendering logic.

A package should have its own manifest, public entry point, and tests. Packages
may depend on other packages, but must not depend on product implementations
such as `desktop`, `mobile`, or `agent`.

The [Remote package development plan](remote-package-development.md) defines
the ongoing Rust/TypeScript protocol, client and server extraction, retaining
independent Crypto and application host adapters. Rust Client extraction has
started; Server security is connected, while TS Client remains planned.
