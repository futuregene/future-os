# Fusion generation budget policy verification

Date: 2026-10-29 (Asia/Shanghai). Follow-up to the [device-run diagnosis](fusion-database-audit.md).

## Current independent-discussion policy (2026-11-02)

Fusion now defaults to independent solution generation. The tool has no candidate-options field; background faithfully compresses context and user requirements with provenance, including user-provided proposals. It excludes the main agent's invented plans and preferences. Constraints are brief material boundaries and may be empty, without language, word-count or formatting rules.

Dynamic capacity admission now replaces the earlier no-override policy. Host shares moderator input capacity across the configured role count after output/task/protocol/JSON reserves, allocating final public-report bytes and a reviewer-safe map independently of working generation tokens. Agent validates every real model turn before sending, including tools/history. Unknown capacities fail closed. No hidden reasoning enters moderator input. Synthesis receives one latest report per role; compact parent output omits conditions, next steps, report and previews. Provider metadata accuracy remains an external assumption. Regression source covers 3/5 role allocation, zero/oversized capacities, actual oversized requests never reaching a provider, current answers and compact results. No paid provider run or local unit execution is claimed for this update.

Regression source covers long inputs, omitted/empty constraints, exclusion of legacy options from forwarded prompts, long synthesis, compact envelopes without previews, legacy fixed budgets ignored on retry, and request-local capacity overrides without mutating live provider settings. Clippy compiles these tests; local unit execution remains delegated to CI. The historical probes below exercised the old bounded policy and do not validate the new one.

## Historical development configuration (2026-10-29)

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

Moderator total generation and forwarded public-answer capacity are independent: mapping may use the model's effective total output budget, while its public JSON must still fit the reviewer allocation. All roles now separate working generation from public-report capacity. Reports are repaired within the original session, with investigation tools disabled for repair; no original task is replayed. No checkpoint recovery is added; users start a fresh discussion after interruption. UI roles are Moderator / Participant N; interrupted moderators display failure while stored interruption metadata remains intact.


## Same-session report submission (2026-11-02)

The device record exposed a remaining participant cap: a 1,000,000-window catalog and 1,748-byte task became a 39,718-byte report allocation after a blanket sixfold escape reserve; dividing by four capped DeepSeek analysis at 9,929 generation tokens, including reasoning. Both attempts reached OUTPUT_LIMIT. A separate moderator attempt timed out at 300 seconds; this change does not claim that timeout was caused by the report cap or is now eliminated.

Working requests now use model capability and actual remaining context, independently of report bytes. The fixed five-turn Fusion limit is removed. Report forwarding is measured in its actual representation without the sixfold reserve. The final report is validated after investigation; oversized/invalid reports receive a tool-free revision request in the same session, sharing the original deadline. Stored step events retain each revision's session/run binding and report_revision; usage remains cumulative. Stage 1 and stage 4 use natural reports, while stage 2/3 retain source/identity validation. Capability 2 fences older Agents that cannot enforce report-only revisions.

Regression source covers 2–5-role generation/report separation, real JSON costs, growing tool histories without counting transport escapes, and oversized/invalid reports repaired through one working session without resubmitting the original task. Desktop paging checks only the final report and excludes drafts/tool-evidence disclosure. These tests are compiled by Clippy/static checks; local unit execution and a new paid provider run are not claimed.


## Output capacity survives the typed model catalog (2026-11-02)

The 17:01:59 device discussion failed in preflight after 177 ms, with no child steps or model dispatch: `Fusion requires a known positive output capacity`. Agent's internal `list_models` JSON included `maxTokens`, but the typed payload/protobuf/codec path discarded it. Strict Host admission then rejected the missing field.

The model entry now carries optional protobuf field 14 (`max_tokens`), exposed as `maxTokens` in the canonical JSON view. Encoding and decoding preserve the Agent's existing `effective_max_tokens()` value. Missing historical fields remain absent; invalid or missing capacities still fail closed, now naming the affected model. No provider capacity is fabricated and no generation/report budget rule is relaxed.

Regression source covers the real Agent handler through typed encoding, protobuf byte serialization/deserialization and the client JSON decoder; Host admission also consumes the typed codec round trip rather than a manually built Host-only catalog. Payload regressions cover presence and older-agent absence. Local static checks compile these regressions; unit execution remains delegated to CI and no paid provider call is claimed.


A separate manual smoke check started the newly built Agent with an isolated temporary FutureOS home, a dummy local provider and a fresh loopback port. The normal verified connection performed readiness/contract checks and read `list_models`. The typed-only response (`data` empty) preserved `contextWindow=262144` and `maxTokens=32768` through the real gRPC client decoder. No model prompt was submitted, no user instance was stopped, and the test process/home and temporary client source were cleaned up. This validates model-catalog transport, not a paid four-stage discussion.
