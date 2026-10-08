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

Deterministic Allow/Deny rules run first. Only Ask requests invoke the classifier.
The classifier uses the existing Future System One endpoint, account credential and
`jev` route shared with skill recommendation. It answers three bounded Choices: risk,
authorization and reason code. The Agent validates the probabilities, derives the unique
maximum when only probabilities are returned, and checks any supplied choice/confidence.
The effective confidence is the more conservative selected probability and confidence.
Provider text never grants execution permission.

| Risk | Authorization required |
| --- | --- |
| low | any |
| medium | medium or high |
| high | high |
| critical | never approved |

The reason catalog has 12 entries. Specific reasons impose minimum risk; scope mismatch
caps authorization at low. Insufficient information produces an uncertain denial.
Approval additionally requires all three confidences at least 0.75. The total deadline
is 30 seconds, including semaphore waits and one transient retry. There are four global
review slots. Cancellation, changed execution context, malformed responses, authentication
and transport errors deny execution. Three denied attempts for the same tool and coarse
target scope in a Run stop further model calls.

Trusted authorization currently uses the current user message. Assistant plans, tool
output, repository text and attachments do not authorize actions. Ambiguous follow-ups
must remain uncertain or weakly authorized. Historical user context is a later improvement.
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
Runs detail shows the decision, risk, authorization, reason, confidence and audit details.

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
