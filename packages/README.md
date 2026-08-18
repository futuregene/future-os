# Shared packages

This directory contains reusable packages consumed by more than one FutureOS
application or service. Package names and public APIs are independent of their
implementation language.

- `rpc`: Rust wire-contract crate and protobuf source of truth.
- `markdown`: Shared TypeScript markdown parser and types.
- `thread-projection`: Shared TypeScript thread projection logic.

A package should have its own manifest, public entry point, and tests. Packages
may depend on other packages, but must not depend on product implementations
such as `desktop`, `mobile`, or `agent`.
