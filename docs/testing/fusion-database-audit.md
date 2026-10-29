# Fusion device-run database audit and progress UI

Date: 2026-10-29 (Asia/Shanghai). Inspected stable local copies of the live Fusion/Agent SQLite files, without changing the original user records. No provider requests were made by this audit.

## Observed device run

The discussion `fus_23b4aa1d…` ran from 17:06:54 to 17:17:42, about 10 minutes 48 seconds. Its parent run completed. Fusion has exactly one parent invocation, one stored tool-result entry and one delivered event. Database identity and restore-generation bindings match; all ten child sessions exist and their actual configured models match the recorded Future models. SQLite quick-check and Fusion foreign-key checks passed.

There are eight logical steps, ten attempts, six successful attempts and four failed attempts. DeepSeek Flash and Qwen 3.7 Plus completed analysis and review; DeepSeek completed issue mapping and synthesis. GLM 5.3 Flash hit the output-token limit three times and timed out on another attempt. The stored `completed` / `partial` combination correctly means a final conclusion was delivered with incomplete participant coverage. It does not mean all three models succeeded. Six issues and twelve initial positions are retained.

The conceptual decision used an empty file read scope, so the empty immutable file manifest is expected. Inner tool activity was limited to snapshot `ls` and `glob`. Parent shell calls occurred before and after the single Fusion call. The longest delays came from GLM's failed attempts and retries under the existing 4096-output-token and 180-second-step limits. One timed-out attempt has no final usage snapshot, so summing stored step costs alone is not a complete cost audit.

## GLM failure diagnosis

The recorded terminal reasons are output truncation and the Host's step timeout. All four child sessions selected `future/glm-5.3-flash` with `thinking_level=default`; the adapter omits thinking controls for that value, leaving the provider's default in effect. Reasoning blocks prove that reasoning was active. The current local model catalog advertises a 128000-token model output capacity, while `agent/src/fusion.rs::LimitedProvider` explicitly caps every inner completion at 4096 tokens. The catalog is current configuration, not an archived wire-request capture.

| Phase / attempt | Duration | Recorded result | Committed output shape |
|---|---|---|---|
| Analysis / first | 142 s | `finish_length`, 4096 output tokens | Reasoning plus a truncated public answer |
| Analysis / retry | 180 s | Host timeout, followed by Agent cancellation | Reasoning, a successful `ls`, more reasoning, no public answer |
| Review / first | 165 s | `finish_length`, 4096 output tokens | Reasoning only, no public answer |
| Review / retry | 126 s | `finish_length`, 4096 output tokens | Reasoning only, no public answer |

Classification used block kinds and byte counts without printing reasoning bodies. The first analysis has 13953 bytes of reasoning and 2942 bytes of public text. The reviews have 16992 and 16795 bytes of reasoning and no public-text blocks. Provider usage does not separate their reasoning-token counts, so byte lengths must not be interpreted as exact token ratios.

The timed-out retry's `ls` returned an empty snapshot listing successfully in about 15 ms, between two model generations. The 180-second timer covers the entire step, including all generations and tool turns, rather than restarting after a tool call. At cancellation the Agent ledger already held partial usage; Fusion had no final usage snapshot. This does not establish the cost of an unfinished provider generation.

`host/src/fusion.rs::with_retry` creates a new session and repeats the same prompt and limits. It does not resume the truncated answer, lower reasoning effort or increase the output budget. Thus the retry repeats the conditions that caused the failure. The evidence supports tuning Fusion's per-model reasoning/output/time policy and making output-limit retries adaptive. Runtime settings were left unchanged in this presentation/audit task; a follow-up should validate any new budget with a bounded provider run before treating it as a fix.

## Presentation change

Desktop/Mobile use the `user-round-group` icon, one four-stage progress indicator and exactly one status row per model. No transcript, issue matrix, diagnostic JSON or raw error is included in the ordinary expanded activity. Full evidence remains in the original database and terminal inspection APIs. Partial completion, timeout and output-limit failures remain visible. Retry grouping changes presentation only, not the stored attempts. The terminal's default `/fusion-log` output uses the same compact stage/status format; a step-specific query still reads original evidence.

Regression cases cover retries, prior-stage failures, transitions, timeout/output-limit classification and parent cancellation. Unit tests are left to CI as required by repository workflow.

## Verification

- Desktop TypeScript and ESLint passed. ESLint retains one existing JSDoc warning in `liveRunProjection.ts`; this change does not edit that file.
- Mobile TypeScript and ESLint passed. The shared projection's regression test source also type-checks.
- `cargo fmt -p future-tui --check` and `cargo clippy -p future-tui --all-targets -- -D warnings` passed, including compilation of the terminal regression cases.
- Documentation placement, bilingual pairs, links, fences and the provider matrix passed `make check-docs`.
- Six browser scenarios passed against the real Desktop and Mobile components: completed, partial/output-limit and running/reviewing on each client. Assertions verify the accessible four-stage progress, current phase, three model names, failure visibility and absence of detailed debug tabs. Mobile uses the Expo Web harness; native Android/iOS installation and Windows/Linux execution remain CI/device checks.

The four-stage indicator reports completed stages and the current stage, not elapsed-time percentage. Source databases were not altered, and no new paid discussion was started.
