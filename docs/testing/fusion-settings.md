# Fusion settings verification

Date: 2026-10-30 (Asia/Shanghai). Configuration contract: [English](../architecture/fusion.md), [中文](../architecture/fusion.zh-CN.md).

The Host owns one moderator and 2–5 participant roles, each with independent model/thinking settings. The shared availability switch gates command completion and backend admission. Changes freeze at consultation admission, not at client render time.

## Manual runtime exercise

A private temporary FutureOS home, workspace and IPC socket were used, copying only the needed Future provider credential. No user database or settings were changed. The real CLI → Agent → Host path saved and executed these roles:

| Role | Model | Thinking |
|---|---|---|
| Moderator | Future DeepSeek Flash | medium |
| P1 | Future DeepSeek Flash | low |
| P2 | Future GLM 5.3 Flash | low |
| P3 | Future Qwen 3.7 Plus | medium |
| P4 | Future DeepSeek Flash | high |
| P5 | Future Qwen 3.7 Plus | low |

The parent requested one focused, discussion-only log-retention decision with an empty file scope. The discussion reached completed/full in 71.784 seconds from admission to the final step, with all 12 logical steps successful on attempt zero. Each recorded step retained the expected role-specific thinking level and 16384-token output cap. Both P4/P5 were accepted in moderator issue references.

During execution, settings were changed to two participants and disabled. The existing five-person discussion completed using the original medium moderator and five-role snapshot. A new `/fusion` request was refused. Saving a one-person panel, an unavailable model, or an unsupported thinking level was rejected without advancing revision. Settings reads retained the saved two-person configuration.

## UI and static checks

The real Desktop, remote Desktop and Mobile components were rendered against the project screenshot harness. Manual scripted interactions verify independent thinking levels for an identical moderator/participant model, saving a two-person panel, preventing removal below two/addition beyond five, refusing empty model rows, and switch persistence. Desktop additionally verifies that disabling removes `/fusion` from command completion. Replay scenarios are in `scripts/screenshots/scenarios.d/fusion-settings.json`.

Rust fmt and Clippy with all targets cover Fusion, Agent, Host, RPC, Remote protocol, CLI, TUI and Desktop Rust. Desktop/mobile TypeScript and ESLint checks cover client code. Remote generated bindings and documentation checks pass. Regression cases cover role/count validation, CLI flags, durable revision checks, frozen settings, disabled admission, actual participant references and duplicate-model progress rows. Unit tests are compiled, not run locally; CI owns execution.

Native Android/iOS interaction and Windows/Linux builds remain device/CI checks. Browser screenshots exercise the actual React/React Native UI with a mocked transport; the isolated provider run exercises the real shared backend.
