# Fusion generation budget policy verification

Date: 2026-10-29 (Asia/Shanghai). Follow-up to the [device-run diagnosis](fusion-database-audit.md).

## Fixed development configuration

`orchestration/fusion/src/config.rs` is the single default-policy source. The same versioned configuration is consumed by the Host and Agent serving Desktop, Mobile and terminal clients. There is no new per-client setting. A future settings page can supply this contract; each admitted discussion already freezes its configuration independently of subsequent defaults.

| Role/model | Thinking level | Initial output cap | Output-limit retry cap |
|---|---|---|---|
| DeepSeek Flash panelist | medium | 16384 | 32768 |
| GLM 5.3 Flash panelist | low | 16384 | 32768 |
| Qwen 3.7 Plus panelist | medium | 16384 | 32768 |
| DeepSeek Flash moderator | medium | 16384 | 32768 |

Each step has 300 seconds across all model and tool turns. The whole discussion has 2700 seconds from creation, enough for the worst-case eight sequential 300-second attempts plus admission/delivery overhead. A retry receives only the remaining total time. These are hard upper bounds, not expected durations. Intermediate answers permit 128 KiB; the compact final tool result remains bounded at 32 KiB.

Output-limit retries increase the token cap and request a shorter, complete answer. Thinking levels remain fixed across attempts. Invalid moderator output receives a format-correction retry. Explicit transient service errors may retry once. Whole-step timeout, cancellation, authentication and unclassified errors do not repeat identically. Empty read scopes expose no file tools.

## Persistence and compatibility

The discussion JSON and created event retain the configuration snapshot. Each step and step event retain actual thinking level, output cap, effective timeout and retry reason. Existing rows without these optional fields remain readable without inventing historical settings or changing the SQL layout version. Agent verifies the recorded child session/run/model and thinking level at execution. Host requires the matching generation-policy capability, so an older Agent cannot silently ignore the larger cap; update both executables together.

## Checks

Regression cases are authored for all fixed roles, increased output-limit retries, unchanged fixed thinking across retries, terminal/permanent failure classification, remaining-time enforcement and legacy record reads. A real HTTP fixture covers the Fusion wrapper and all three Chat reasoning formats, proving the output cap and effort reach the wire without changing ordinary provider settings. These unit tests are compiled by Clippy but are not run locally; CI owns execution.

Two bounded GLM provider probes used low thinking and 16384 output tokens, without file tools or access to user discussion content:

| Probe | Duration | Output tokens | Finish reason | Public answer |
|---|---|---|---|---|
| Independent analysis | 39.08 s | 1131 | stop | 2099 characters |
| Cross discussion | 11.19 s | 442 | stop | 718 characters |

The provider reported zero reasoning tokens for these two probes. This observation does not guarantee how every prompt will allocate reasoning or how long a later discussion will take. Both probes are smaller than the original failed discussion input.

The isolated full 3→1→3→1 exercise passed. A separate temporary FutureOS home, IPC socket and workspace were used, with only the needed Future provider credential copied into private temporary storage. The parent used the normal CLI/Agent tool path and one focused report-tool decision, with an authorized `facts.txt` snapshot.

- Parent completed successfully in 113.39 seconds, with exactly one Fusion discussion.
- Fusion reached `completed` / `full`; all eight steps completed on attempt zero.
- Every step recorded the expected explicit thinking level, 16384-token output cap and 300000 ms timeout.
- GLM completed analysis in 34.70 seconds and cross discussion in 20.19 seconds, reporting 771 and 536 output tokens respectively.
- Issue mapping and synthesis both completed through the fixed DeepSeek moderator.
- Original `facts.txt` SHA-256 was unchanged. No implementation or file mutation was requested.
- This is a new bounded exercise, not a replay of the user's original prompt and full context. It proves this configuration works on the exercised path, not a universal latency guarantee.

Static verification passed `cargo fmt --check` for changed crates, `cargo clippy --all-targets -- -D warnings` for Fusion/Agent/Host/CLI/TUI and the Desktop Rust consumer, and `make check-docs`. Unit tests were not executed locally. Windows/Linux builds and native Android/iOS execution remain CI/device validation; all three clients consume the same backend policy.
