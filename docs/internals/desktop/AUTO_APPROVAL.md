# Automatic approval: implementation and policy

Status: first implementation, under test and integration. Production false-approval rates
require independent evaluation. The detailed policy and original baseline analysis are in
[the Chinese design](AUTO_APPROVAL.zh-CN.md).

## Modes and execution

The product modes are manual, sandbox, auto and off. `SandboxTier` retains its three
existing values. Auto sends `tier=sandbox, reviewer=model`; unsupported hosts use manual
review while retaining the user's automatic preference. A Desktop checks the Agent's
reviewer acknowledgment before accepting automatic mode. Remote mobile clients offer
this mode only when the Desktop advertises `auto_approval_v1`.
Automatic review requires a signed-in FutureOS account. Composer and Settings disable
it while signed out, invalid, or checking sign-in. A saved automatic preference is
persistently downgraded to sandbox on confirmed sign-out or invalid credentials; signing
in again does not restore it. Temporary account verification failures retain the preference.
Desktop settings writes and prompt setup also check for the Future credential. Agent policy
configuration and queued/historical run snapshots remove the model reviewer when the
credential is absent, preserving the OS sandbox and the existing human approval flow.
If the sandbox is unavailable after that downgrade, the existing Manual fallback applies.

Deterministic Allow/Deny rules run first. Only Ask requests invoke the classifier.
The classifier uses the existing Future System One endpoint, account credential and
`jev` route shared with skill recommendation. It answers three bounded Choices: risk,
authorization and reason code. The Agent validates the probabilities, derives the
maximum when only probabilities are returned, and checks any supplied choice/confidence.
Rounded ties are valid distributions but cannot meet the approval confidence threshold;
any supplied choice must still be a maximum. Gateway `id` is retained as the provider request ID.
The effective confidence is the more conservative selected probability and confidence.
Provider text never grants execution permission.

| Risk | Allowed authorization probability (≥ 0.75) | Other confidence required (≥ 0.75) |
| --- | --- | --- |
| low | P(low) + P(medium) + P(high) | risk and reason |
| medium | P(medium) + P(high) | risk and reason |
| high | P(high) | risk and reason |
| critical | never approved | — |

The reason catalog has 12 entries. Specific reasons impose minimum risk; scope mismatch
caps authorization at low. Insufficient information produces an uncertain denial.
Policy version 2 measures authorization support by summing the permitted outcomes of
the validated probability distribution; Unknown is excluded for every risk level.
The native confidence describes certainty in an individual authorization label and is
preserved for audit, without being used to scale this combined probability. The sum is
stored as `confidence.authorization_support`. Thus a distribution of High 49%, Medium
19%, Low 31%, Unknown 1% supports Low-risk approval at 99%, but supports Medium at 68%
and High at 49%, which remain uncertain denials. Reason risk floors and scope mismatch
denials run first, and risk/reason confidence still must reach 0.75. The total deadline
is 30 seconds, including semaphore waits and one transient retry. There are four global
review slots. Cancellation, changed execution context, malformed responses, authentication
and transport errors deny execution. Three denied attempts for the same tool and coarse
target scope in a Run stop further model calls.

Trusted authorization uses the current user message and an immutable snapshot of up to
16 preceding original user messages, in chronological order, within a total 32 KiB text
budget. Later instructions override earlier permissions. Only the visible first text
block is included: assistant plans, tool output, model compaction summaries, injected
sidecars, repository text and attachments do not authorize actions. History is a contiguous
suffix of whole messages; omitted history is flagged and unresolved references remain
uncertain. Prompt version 3 treats an ordinary new filename or harmless sample text
as delegated implementation details when the user explicitly requests a test file and
leaves these unspecified. Explicit names, content, revocations and restrictions still
govern; sensitive data, recipients and destructive targets require exact authorization.
Omitted file bodies alone do not prove an authorization gap. Reason catalog version 2
distinguishes ordinary local user-file actions outside the workspace from remote writes.
Whole-command sandbox escape retains the existing sandbox implementation and is improved
with the sandbox separately; it is not an automatic-approval prerequisite.

## Feedback and audit

Automatic review emits `approval_assessment`, never an interactive `approval_request`.
Runs remain running while the main model selects its next action. Host feedback is kept
in tool-message metadata and lowered by all three provider adapters separately from the
persisted tool output. Audit input omits file bodies and environment values, redacts common
credential patterns and URL queries, and binds exact executable arguments using SHA-256.

The Desktop transaction inserts a terminal `approval_requests` row with reviewer `model`,
source `auto_review` and scope `once`, plus an immutable `approval_assessments` row.
Review errors/uncertainty become rejected queue records; their precise status remains in
the assessment. The table indexes run/time; JSON payload contains reported/effective
classification, probabilities, confidences, sanitized action, digest, versions, model,
provider request identity, duration and stable error code. Replay is idempotent and foreign
keys cascade cleanup. Fresh and upgraded databases use migration `v1.2.2-auto-approval`.
Runs detail shows the decision, effective risk and authorization, with a short localized
explanation when an action was not run. It appears after the basic run/tool information and before results, using the existing
tool metadata card styles.
Probabilities, confidence scores, raw actions, model identity and audit versions remain in
the database for diagnosis and are not displayed in the product UI. Historical decisions
are never recalculated or rewritten.

## Code and validation

- `agent/src/system_one.rs`: shared gateway transport and Choice builder.
- `agent/src/approval_review/`: classifier adapter, deterministic policy and fail-closed lifecycle.
- `agent/src/rpc/approval.rs` and `session_prompt.rs`: per-Run Ask routing.
- `packages/rpc/proto/future.proto`: additive reviewer field and typed assessment event.
- `desktop/src-tauri/src/store/approval_assessments.rs`: terminal audit projection.
- `desktop/src/features/runs/ApprovalAssessments.tsx`: Runs inspection.

Tests cover the decision matrix, reason corrections, malformed probability responses,
probabilities-only gateway compatibility, bounded retries with stable request identity,
cancellation/stale requests, denial limits, Allow/Deny bypass, provider feedback, protocol
round trips, terminal persistence, replay, migration idempotence and rollback.

Production rollout still needs bilingual approval evaluation data, confidence calibration,
critical protected-scope configuration, model-version policy and live integration testing.
The first defaults are implementation choices, not claims of measured model safety.
