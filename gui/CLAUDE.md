# GUI Development Guide (`gui/`)

FutureOS desktop app: Tauri + React + TypeScript, frontend `src/`, Tauri backend `src-tauri/` (Rust), connects to the repo-root agent via gRPC. For overall monorepo architecture/build, see **repo-root `CLAUDE.md`**; this file covers `gui/` only.

## Document Map (read relevant sections on demand — don't pull whole files into context)

> Development docs live under `gui/DEV_MD/`. Paths below are relative to `gui/`.

| Document | Content | When to Read / Modify |
|---|---|---|
| `DEV_MD/PRODUCT.md` (~21KB) | Product positioning, module boundaries, workspace object semantics, desktop experience | **Read** when changing product behavior / adding features / confirming domain semantics; **modify** only when product decisions change |
| `DEV_MD/ER.md` (~33KB) | Data objects & relationships, table inventory, schema design decisions | **Read** when changing store / data flow; **modify** and keep in sync when schema changes |
| `DEV_MD/COLOR.md` (4KB) | Color semantic tokens + quick usage reference | **Read** when picking colors / changing styles; **modify** only when adding/changing tokens |

> `DEV_MD/PRODUCT.md` / `DEV_MD/ER.md` are large: use `Read` with `offset/limit` to read **specific sections** from the chapter index below — don't load the whole file.

### Chapter Quick Reference
- **PRODUCT.md**: §1 Positioning · §2 Module Boundaries · §3 Product Principles · §4 Work Objects (4.1 Workspace / 4.2 Chat / 4.3 Message / 4.4 Run / 4.5 Tool / 4.6 Approval / 4.7 Review / 4.8 Artifact / 4.9 Research / 4.10 Data / 4.11 Skill / 4.12 Attachment) · §5 Desktop Experience (5.1 Three-panel / 5.2 Left Nav / 5.3 Chat Area / 5.4 Right Context / 5.5 Colors / **5.6 Settings: Provider/Model/Login**) · §6 Agent Workflow · §9 Roadmap
- **ER.md**: §2 Relationship Overview · §3 Naming Conventions · §4 Objects (4.1 Workspace … 4.8 Approval Request / 4.9 Review Changeset / 4.10 Review File Change (incl. **Shadow Review extension**: `review_snapshots` table + changeset/file_change extension columns) … 4.20 Object Reference) · §5 V1 Table Inventory · §6 Key Design Decisions (**6.8 Shadow Repo "Previous Change Set"** / **6.9 Provider/Model/Login Config**)

> **Shadow Review** (run-level "previous change set"): product semantics in DEV_MD/PRODUCT.md §4.7, data model in DEV_MD/ER.md §4.10, design tradeoffs in DEV_MD/ER.md §6.8. Read all three before modifying shadow repo / snapshot / changeset code (`src-tauri/src/shadow_review/`, `store/review_snapshots.rs`).

> **Provider / Model / FutureGene Login**: product behavior in DEV_MD/PRODUCT.md §5.6, storage & login implementation in DEV_MD/ER.md §6.9, field validation in DEV_MD/PLAN.md "Custom Provider Field Validation". Read these before modifying `agent_providers.rs` / `auth_store.rs` / `future_login.rs` / `commands/login.rs`.

## Code Structure (`src/`)

- `components/layout/` — `AppShell` (layout orchestration) + `ContextPanel`; `hooks/` contains AppShell domain hooks: `useThreadStore` / `useAgentConnection` / `useApprovals`
- `components/ui/` — Generic presentational components (`Badge` / `DiffView` / `CopyablePre` / `TextInput` / `Select` / `Button` / `Overlay` …), no business logic
- `features/{agent,review,runs,artifacts,research,settings,markdown}/` — Domain-specific business components
- `integrations/` — Boundary with the Tauri backend: `tauri/invoke.ts` (sole typed invoke entry point), `agent/`, `storage/` (`threadStore.ts` is the barrel, domain modules in sibling directories)
- `lib/` — Dependency-free utilities: `usePolling` / `useAsyncResource` / `futureEvents` (typed event bus) / `cn` / `clipboard` / `date` / `platform` / `useDismissableLayer` / `windowDrag`

## GUI Development Principles (Long-term Memory)

1. **Colors**: Use only semantic tokens from `DEV_MD/COLOR.md`; no bare Tailwind colors (`blue-300`…). Status badges use `<Badge tone>`; categorical colors (event categories / error subtypes) are intentional exceptions.
2. **Tauri Invoke**: All `invoke` calls go through `integrations/tauri/invoke.ts`'s `invokeCommand` — never call `invoke` directly. Command params: structured input via `{ input }`, single scalars via named keys. (Other `@tauri-apps/api` capabilities — event `listen`, dialog, `convertFileSrc`, window/webview — are not wrapped by `invoke.ts`; import them directly as needed.)
3. **Cross-component Events**: Use `lib/futureEvents.ts` typed `emitFutureEvent` / `onFutureEvent`; never use raw `window` CustomEvent.
4. **Async / Polling**: Cancellation-safe loading uses `lib/useAsyncResource`, polling uses `lib/usePolling` (don't hand-roll `cancelled` flag effects or `setInterval`). When polling connection/status changes, **don't flash `checking` on every tick** — retry silently, only update state when you have a result.
5. **AppShell State**: Split by domain into hooks under `components/layout/hooks/`; AppShell only does layout orchestration. Hooks expose state via named destructuring to AppShell to minimize the change surface.
6. **Data**: Schema changes must sync to `DEV_MD/ER.md`; frontend store changes must account for corresponding backend `src-tauri/src/store/` (split by domain).
7. **Backend Errors**: Tauri commands return `Result<_, AppError>` (`thiserror`), **serialized as strings**; frontend handles them as strings. Backend loose colors have been tokenized / AppError'd — don't regress to `.map_err(|e| e.to_string())`.
8. **Approval (v2 file-based + three-tier)**: Approval targets **file path access**. Rules stored in `${WS}/.future/approval_rule.json` and `~/.future/approval_rule.json`; the agent reads them directly. GUI writes via trusted path proxy (`approval_rules.rs` + `commands/approvals.rs`). Three tiers (`app_settings.approval_tier`: `manual` / `sandbox` (macOS only) / `off`), dispatched at session establishment via `set_sandbox_policy { tier }`. The `approval_requests` table retains structured `action_payload` / `sandbox_boundary` / `save_suggestion` fields to power approval cards. Semantics: `DEV_MD/APPROVAL_PLAN.md` / `DEV_MD/SANDBOX_PLAN.md` / `DEV_MD/ER.md §4.8`. (Legacy P2 `approval_config.rs` scaffolding and three reserved tables were deleted on 2026-07-05.)

## Verification (Run After Every Change)

```bash
cd gui && npx tsc --noEmit && npx eslint "src/**/*.{ts,tsx}" && npx vitest run
# If Tauri backend is affected, also run:
cd gui/src-tauri && cargo fmt --check && cargo clippy && cargo test
```

GPG signing fails in non-interactive terminals; commit with `git commit --no-gpg-sign`. Visual changes (colors, etc.) must be confirmed in a live `make run-gui`.
