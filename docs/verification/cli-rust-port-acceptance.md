# cli-rust-port — Final Acceptance Report (P4-final)

Status: **ACCEPTED** — verified 2026-08-07 on commit `1069f227` (branch `claude/cli-rust-port`,
merge of `origin/main` incl. PR #112 typed-RPC wire contract).
Scope: 1:1 TypeScript→Rust port of `cli/` (TS) → `cli/` (Rust crate, bin `future`);
the TS CLI was retired in this PR and `cli/rust/*` was promoted to `cli/`.
byte-identical args / help text / output / exit codes, with ported unit tests + a
golden test harness (Rust CLI vs recorded TS goldens).

> Post-merge note (PR #112 integration): `cli/` now consumes the generated proto
> code from the `future-rpc` crate (the single proto-codegen owner introduced by #112)
> instead of owning a generated copy — `src/generated/` and the proto half of build.rs
> were removed; `rpc.rs` imports `future_rpc::proto`. Verified wire-compatible against
> the new typed-payload agent (163/163 diff, incl. the live-gRPC `agent` scenario).

---

## 1. Verification results (all gates green)

| Gate | Command | Result |
|---|---|---|
| Golden harness (Rust vs recorded TS goldens) | `make test-cli-diff` (`cli/tests/diff-ts-rust.sh`) | **163 passed / 0 failed / 0 skipped** |
| cli-rust unit tests | `cargo test -p cli-rust` | **194 passed / 0 failed** |
| Workspace tests | `cargo test --workspace` | **1729 passed / 0 failed** (incl. future-rpc) |
| Workspace clippy (CI flags) | `cargo clippy --workspace --all-targets -- -D warnings` (rustup 1.97.0) | clean |
| Format | `cargo fmt --check` | clean |
| TS CLI typecheck | `npx tsc --noEmit` (via `make lint-cli`) | clean (unchanged TS) |

The differential harness rebuilds **both** CLIs with `FUTURE_VERSION=0.0.0-diff+local`
(the TS build mirrors `make build-cli`: `npm run gen-version` + `npm run build` +
`bun build --compile`), runs both from isolated `$WORK` copies (no `future-agent`
sibling, immune to concurrent rebuild clobbering), sanity-checks the baked version
first, and compares stdout / stderr / exit code byte-for-byte per argv.

### Corpus inventory (163 cases, 15 scenarios)

| Scenario | Cases | Coverage |
|---|---|---|
| `static` | 55 | all help texts (both auth group-help variants), `--version`/`-v`/`version`, bogus groups, run parse quirks (no-prompt exit 1, invalid-flag exit 0) |
| `home:none` / `home:auth` / `logout` / `home:badjson` | 13 | auth status/credential/logout, account profile/balance over auth.json states (incl. corrupt JSON fallback) |
| `http` | 26 | account profile/balance, `auth login --url`/`--url=`, tools list/describe/call against mock MCP (SSE) incl. validation + error translation, `init` (empty catalog) |
| `http:errors` | 2 | 401 error-body translation |
| `init:linked` / `init:blocked` | 2 | idempotent re-link + blocked `.future/bin/future` paths |
| `agent` | 17 | real `future-agent` on a dedicated gRPC port: agent status/models/session list/info/rename/delete (+`--json`) |
| `doctor` | 1 | fully controlled env (fake bin dir, dead gRPC) |
| `skills` / `skills:installed` | 13 | list/install/install-builtin/uninstall/update over mock catalog + deterministic download zips |
| `agentdown` | 2 | dead gRPC port, **stderr-prefix** mode (see §2) |
| `browser` | 32 | mock CDP endpoint + scripted WebSocket: status/start(already-running)/tabs(select/new/close)/open/snapshot/click/type/press/screenshot/scroll/console |

Cumulative corpus growth: 90 (P4) → 131 (+P2 run/tools/skills/mcp) → 163 (+P3 browser CDP).

---

## 2. Final divergence list (known / accepted)

Byte-identity holds everywhere except transport-error *wording* (network-stack
dependent, same class as the P4 turn-1 findings). All are documented in
`cli/src/rpc.rs` (module doc) and the harness header.

1. **gRPC transport error text** — tonic ("transport error") vs grpc-js
   ("14 UNAVAILABLE: …") when the agent is down. Only the agent-down paths are
   affected; the harness's `agentdown` mode compares exit code + stdout bytes +
   stderr **prefix** (`Error:`) for these two cases. Everything else is `exact`.
2. **HTTP transport error text** — reqwest vs node-fetch wording for an
   *unreachable* `--url`. Not exercised in the corpus (all `auth login --url`
   cases point at the reachable mock); excluded by design.
3. **`browser status` against an unreachable endpoint** — Bun ("Unable to
   connect…") vs reqwest wording. The corpus covers the fixed-message
   `ensureBrowser` error via `snapshot --endpoint http://127.0.0.1:1` instead
   (byte-identical), and the already-running `status` path (byte-identical).

### Excluded from the corpus by design (documented in harness header)

- `future run` against a live agent — `agent_end` carries variable
  `duration_ms` / event ids (not deterministic); local-only run paths + the
  dead-agent transport error ARE covered.
- `browser start` spawning a real Chrome — non-deterministic; only the
  already-running path is diffed.
- Live-network skill downloads — replaced by deterministic mock zips.

---

## 3. What shipped (commits on `claude/cli-rust-port`, ahead of origin/main)

- `10462696` P0 scaffold: dispatch 1:1 of `index.ts main()`, verbatim help texts
  (both auth group-help variants), predicates + stubs, utils/constants/types,
  Output sinks, tonic/prost gRPC client, 17 unit tests.
- `c891de5a` + `57b59922` P1: init/auth/account/models/agent status/session/doctor
  over rpc.rs + reqwest; auth.json helpers; test_env ENV_LOCK; help golden test;
  `preserve_order`; 58 unit tests.
- `0092147c` + `e79bb960` (+`caebf63d` scaffold) P4: diff harness + mock platform
  server; make targets `test-cli-diff`; 90/90.
- `d7bc948f` P2: run/tools/skills/mcp bodies (streaming events, SSE MCP, tool
  catalog, error translations, browser tool surface + config v1→v2); 100 unit
  tests; corpus 131/131.
- `99ebcaab` P3: browser subsystem — chromium CDP WebSocket session backend +
  Safari path + selector/input/screenshot; corpus 163/163.
- `d5fa284a` corpus refinement: `snapshot --endpoint <unreachable>` covers the
  fixed-message ensureBrowser error (status transport wording diverges).
- `1069f227` merge of `origin/main` (#112 typed-RPC): cli consumes
  `future-rpc` crate (generated proto retired), Makefile `test-cli-diff`
  gains the `node-workspace` prerequisite (root npm install + future-rpc/ts
  build for the TS side), acceptance numbers refreshed (1729 workspace).

## 4. Re-running the gates

```bash
make test-cli-diff            # ~5-7 min (rebuilds both CLIs)
make test-cli-rust            # cargo test -p cli-rust
rustup run 1.97.0 cargo clippy --workspace --all-targets -- -D warnings
rustup run 1.97.0 cargo test --workspace
```

Note: Homebrew cargo ignores `rust-toolchain.toml` — always run under
`rustup run 1.97.0`. Unset/override leaked `FUTURE_VERSION` when checking
`--version` output outside the harness.
