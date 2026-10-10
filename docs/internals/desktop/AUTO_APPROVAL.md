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
Rounded ties are valid distributions; risk/reason ties cannot meet their confidence threshold.
Authorization can pass through the combined permitted probability mass;
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
is 30 seconds, including action fingerprinting, evidence preparation, semaphore waits and one transient retry. There are four global
review slots. Cancellation, changed execution context, malformed responses, authentication
and transport errors deny execution. Only completed negative model judgments (`rejected` or `review_uncertain`) count toward
the three-attempt limit within a Run. Configuration, transport, timeout and budget errors
do not consume it. The bucket binds the tool, operation kind, cwd and exact confirmed
targets; with no confirmed target it binds executable parameters, excluding review prose
and call IDs. Changing the call ID alone does not reset the limit. This is not semantic
command equivalence: distinct command strings without confirmed targets have separate buckets.

Sandbox-failure routing recognizes lowercase Node `operation not permitted` and
structured `EPERM` errors on macOS. A matching nonzero failure can request review
and retry within the same tool call. Permission summaries share the diagnostic
recognizer; diagnostic paths remain untrusted. The shell schema advertises
`additional_permissions` only on Windows, while unsupported inputs remain rejected.

Prompt version 5 uses four source-attributed evidence sections (state schema 3):

- `action`: the pending tool request, identified by its tool-call ID. Commands stay whole;
  file bodies and environment values are omitted, and credentials are redacted.
- `trusted_context`: the current original user message and a chronological, contiguous
  suffix of whole preceding original user messages. Later restrictions override earlier
  permissions. Only the first visible text block is used; injected sidecars, attachments
  and model compaction summaries do not authorize actions.
- `host_facts`: host-derived targets, execution boundary, scope, behavior and Ask result.
  Network enforcement is explicitly `unrestricted`: the sandbox does not intercept network
  requests, and network activity alone does not trigger this review.
- `untrusted_context`: adjacent assistant questions, existing tool calls/results, and
  escalation justification/failure summaries. Source IDs, kinds, order and tool-call links
  preserve provenance; these records can clarify references but never grant authorization.
  Tool arguments are projected; reasoning, provider metadata and all tool-result bodies
  are omitted. Results/failures supply fixed status, output byte length, bounded diagnostic
  categories and shell exit code. Diagnostic path mentions remain untrusted, explicitly
  incomplete context; they never become confirmed host targets or workspace membership.
  The run save callback collects evidence for both persistent and ephemeral runs.

The selector preserves mandatory action, current user message and host facts without
truncation. It admits the immediately preceding assistant question, then whole user history
from newest to oldest, stopping at the first message that cannot fit. Remaining space admits
current diagnostic paths/failure summaries, related tool evidence, justification and adjacent
assistant context in that order. Long model justification cannot evict user restrictions.
It records omission counts and excluded compaction checkpoints in `coverage`; missing material facts remain the model's judgment
through `unknown` / `insufficient_information`. There is no `evidence_status` or new semantic
approval gate. Oversized mandatory input produces `review_error / input_too_large` before
calling the provider, retaining size/source audit metadata. The three Choice questions, policy matrix and 0.75 threshold are unchanged.

Input selection targets 6,000 estimated state tokens, with hard caps of 8,000 for state,
4,000 for all questions and 12,000 for the whole serialized request. It also checks 30,000
for state plus the longest question and 62,000 for state plus all questions, leaving headroom below the [official Jev limits](https://docs.typesafe.ai/models). With no Jev
tokenizer, the conservative estimator charges each serialized UTF-8 byte as one token;
these are estimates, not actual tokenizer counts or guarantees about gateway-added text.
Framing, escaping and source metadata count toward the budget. The in-memory history and
background buffers each retain at most 32 KiB of serialized records, with cached sizes.
Oversized original prose is omitted before copying/redaction; mandatory raw inputs over
32 KiB are rejected before evidence serialization. Tool diagnostics inspect only the last
2,048 UTF-8 bytes without forwarding any of that text. `redacted_text_bytes` and retained
ranges describe projected, redacted text; `output_bytes` describes the original tool body. Exact execution digests stream canonical
JSON without copying file bodies; oversized commands are rejected before redaction.
Unknown shell targets retain null workspace membership, even with a workspace cwd.

Ordinary unspecified new filenames or harmless sample text remain delegated details;
sensitive data, recipients and destructive targets require exact authorization. Omitted
file bodies alone do not prove an authorization gap. Reason catalog version 2 distinguishes
ordinary local user-file actions outside the workspace from remote writes. Whole-command
sandbox escape retains the existing sandbox implementation and is improved separately.

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
provider request identity, duration and stable error code. `input_context` records the
selected source IDs, coverage, section sizes, clipping metadata, budget estimates and a
SHA-256 digest of the redacted state, without storing original evidence text. The additive
RPC field preserves these records through Desktop audit persistence. Replay is idempotent and foreign
keys cascade cleanup. Fresh and upgraded databases use migration `v1.2.2-auto-approval`.
Runs detail shows the decision, effective risk and authorization, with a short localized
explanation when an action was not run. It appears after the basic run/tool information and before results, using the existing
tool metadata card styles.
Probabilities, confidence scores, raw actions, model identity and audit versions remain in
the database for diagnosis and are not displayed in the product UI. Historical decisions
are never recalculated or rewritten.

## Code and validation

- `agent/src/system_one.rs`: shared endpoint configuration, transport and Choice builder.
- `agent/src/approval_review/`: separate action, evidence, budget, redaction, prompt, policy,
  provider, lifecycle and typed-result modules; regression tests grouped by responsibility.
- `agent/src/rpc/approval.rs`: per-Run Ask routing and typed review outcomes; card shaping,
  escalation parsing and tests live in `agent/src/rpc/approval/`. `session_prompt.rs` delegates
  reviewer construction to the gate and forwards saved evidence.
- `packages/rpc/proto/future.proto`: additive reviewer field and typed assessment event.
- `desktop/src-tauri/src/store/approval_assessments.rs`: terminal audit projection.
- `desktop/src/features/runs/ApprovalAssessments.tsx`: Runs inspection.

Tests cover the decision matrix, reason corrections, malformed probability responses,
probabilities-only gateway compatibility, bounded retries with stable request identity,
cancellation/stale requests, denial limits, Allow/Deny bypass, provider feedback, protocol
round trips, terminal persistence, replay, migration idempotence and rollback.

Production rollout still needs bilingual approval evaluation data, confidence calibration,
critical protected-scope configuration, model-version policy and live integration testing.
Literal redaction is best-effort and is not a shell parser or a guarantee that arbitrary
command text contains no secrets. Commands stay whole; arbitrary tool bodies are excluded.
The first defaults are implementation choices, not claims of measured model safety.
