# Fusion resilience verification

Date: 2026-10-30 (Asia/Shanghai). Policy: generation 2, resilience 1. See the [design](../architecture/fusion.md) and [Chinese design](../architecture/fusion.zh-CN.md).

## Isolated orchestration exercises

A newly built unified `future` binary ran the shipping foreground Host and Agent, using private temporary FutureOS homes, IPC sockets and workspaces. Explicit keyless custom providers pointed only at a local OpenAI-compatible HTTP/SSE fixture. The outer model called the real `fusion` tool through `future run '/fusion …'`, received its result and completed. No user credentials, real provider requests or user data were used. These were manual application exercises, not local execution of the unit test suite.

The first exercise passed eleven scenarios. Expanded exercises verified the following twenty-two distinct scenarios, including both normal and degraded tool outcomes. A missing-public-text response initially exposed an untyped error; it was corrected to `invalid_output` and rechecked with two bounded attempts and no participant dispatch.

| Scenario | Observed behavior |
|---|---|
| Healthy three-role discussion | `completed/full`; one preflight and eight discussion attempts; six public materials retained |
| One first-round role forbidden | `completed/partial`; analysis and review include only P1/P3; failed P2 is not called in review |
| Two first-round roles forbidden | `completed/single`; only P1; mapping/review skipped; moderator still performs synthesis |
| All first-round roles forbidden | `failed/none`; no mapping, review or synthesis; no invented conclusion |
| Moderator preflight HTTP 401 | `failed/none`; participant requests never sent; no model substitution |
| Moderator synthesis HTTP 403 | `failed/none`; six completed public analyses/reviews retained; parent receives `has_conclusion=false`, `conclusion=null` |
| Mapping output invalid twice | Two retained failed attempts; `direct_review`, followed by successful partial synthesis |
| Provider HTTP 429 insufficient quota | Classified as quota, not rate limit; no identical retry; queued affected roles skipped; no moderator substitution |
| Single-role transient HTTP 503 | One bounded retry; completed full coverage; original failed attempt remains in storage |
| Parent cancellation | Discussion remains cancelled without a conclusion; owned child attempts stop; parent is not resumed |
| Preflight cache and profile change | Repeated matching profile reuses success; changing moderator thinking level forces another readiness request |
| Moderator mapping HTTP 403 | Stops before review/synthesis; three analyses retained; no fallback moderator |
| Invalid synthesis twice | Two failed format attempts; six materials retained; failed result remains without a conclusion |
| One review forbidden | Valid first-round analyses retained; review includes P1/P3; partial synthesis |
| All reviews forbidden | `analysis_only`; three independent analyses retained; partial synthesis explicitly lacks cross-review |
| Ordinary HTTP 429 rate limit | One retry, followed by complete coverage; not confused with quota failure |
| Two distinct models HTTP 503 | Provider cooldown opens; queued role skipped; all-invalid analysis stops without synthesis |
| Empty readiness public output | Two `invalid_output` attempts, then failure; participants never dispatched |
| Summary time reserve | Private fixture uses 3-second steps/9-second run; mapping is cut off at the reserve, review skipped, synthesis completes as partial |
| One configured participant missing | P2 skipped; P1/P3 complete both rounds with original IDs |
| Configured moderator missing | No inner HTTP model requests; moderator and panel roles skipped explicitly |
| All configured participants missing | No inner HTTP model requests, including no paid readiness check |

Every readiness request exposed no tools. All inspected inner requests had only the permitted snapshot-tool subset (empty with empty read scope). The parent retained its ordinary tools plus Fusion. Recorded thinking effort matched each role. Every moderator phase used the configured moderator reference; no participant was promoted.

The fixture compared HTTP request history with `future fusion show --json`, step attempts, typed failures, completed/skipped phases, active role IDs, retained material counts and the actual tool envelopes observed by the parent. The quota rerun confirmed the queued third role was skipped before HTTP dispatch. Successful attempts remain available even when later moderator work fails. Ordinary failed outcomes report completeness `none`; they cannot retain the old `full` admission default.

## Durable regressions and client projection

Regression tests are authored for quota versus rate-limit classification, model-scoped 403, transient-code precedence over generic timeout wording, summary reserve, UTF-8 truncation, evidence-reference preservation under the 32 KiB envelope cap, atomic failure/report persistence, legacy rows without report/configuration, cancellation fencing and a committed conclusion winning over a stale timeout observer. Host tests cover profile/Agent scope separation, one model failing twice versus two distinct failed models, and provider cooldown scope. Shared TypeScript and TUI cases cover typed timeout/skipped states, skipped-stage progress and explicit no-conclusion/material-retention notices.

Desktop and Mobile consume the same TypeScript projection. TUI maps the same stored report codes and typed step failures to matching English/Chinese copy. Preflight is shown as checking the main model, followed by the existing four-stage progress display. Skipped segments are neutral; failed participants remain visible after later phases. Compact notices describe reduced participation, single-role coverage, reserved synthesis time, direct review and missing conclusions without dumping raw provider errors.

## Validation boundaries

Scoped Rust fmt and Clippy include all targets for Fusion, Agent, Host, CLI, TUI and the Desktop Rust consumer. Desktop/Mobile TypeScript and their project ESLint checks validate the UI consumers. Documentation placement, English/Chinese pairing and links use the repository docs gate. Unit tests are compiled by Clippy but are not executed locally, per repository policy; execution belongs to CI.

The manual runtime exercises ran on macOS. Windows/Linux compilation and native Android/iOS device interaction remain CI/device validation. The three clients share one backend policy; this record does not claim device runs that were not performed. The 60-second readiness observation does not guarantee later provider health; monetary budgets, finer multiple-account routing and automatic moderator replacement remain outside this batch.
