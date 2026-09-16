# Native local/API open-book replay v2 (approved after v3 closed book)

## Amendment after the first instrumented run

The first native run is retained under `native-open/run-v1`, but is NOT qualified
as the formal comparison: its study guard rejected normal `rg -e/-c`, sort/uniq
and head syntax, and optional retrieval was often unused. Its CNY 0.92729196 is
retained in the shared budget. V2 admits these ordinary read-only forms, pipes,
`&&` and semicolon-separated validated readers. Arbitrary scripts, execution
hooks, foreign paths, permission escalation and writes remain forbidden.

V2 explicitly requires original-archive checking before a final answer. The
first successful native invocation is requested with `tool_choice=required`;
subsequent decisions remain model-driven. The `native_invoked` metric proves a
native tool ran, not by itself that the right evidence was read; final review
must inspect traces. This is a required-verification condition, not a claim
about products' default willingness to retrieve. Do not mix its scores with v1.
Opening cumulative spend is CNY 42.08420432; the total ceiling remains CNY 300.

## Scope and authorization

Use the unchanged v3 projections, questions, six corpora and 18 cutoffs. The user
approved native **history-recovery mechanism replay**, not reconstruction of the
original products' entire production machines. No current project files are
used as historical evidence. No hosted Codex history/notes API is simulated.
The examiner remains `future/deepseek-flash`, thinking disabled, 8192 output
cap. Total admission ceiling is CNY 300 including prior CNY 41.15691236.

## What actually runs

- **C/C3:** the checked-out Future CLI + isolated native Agent database and
  history search/get RPC implementation. An offline provider placeholder
  prevents model calls from the archive agent. Original normalized message
  entry IDs are preserved. A fresh HOME and TCP port isolate each case.
- **Codex:** unmodified upstream CLI built from b13164d8. Local rollout fixtures
  are validated by the upstream `codex_rollout::parse_rollout_line` function.
  The original exec_command/write_stdin handlers execute each read. A local
  deterministic Responses transport injects the EXAMINER'S requested tool call
  and captures the native result. It does not search, answer, or supply fake
  tool results. The native exec process persists throughout one exam so its
  exec-session IDs and polling remain real. Named native permissions grant
  only platform runtime packages, case workspace and case rollout reads;
  writes, external case reads and network access are denied. Hosted history
  tools are not enabled in this third-party/API mode.
- **OpenCode:** e03db9bc, using its pinned Bun 1.3.14. Original CLI import/export
  validates the native message store by complete parts roundtrip. Original
  `/experimental/tool` supplies schemas; `debug agent build --tool ...` invokes
  the real ToolRegistry handlers, native output bounds and native permissions.
  Explicit allow/deny avoids that debug entrypoint's auto-approval of `ask`.
  External/Claude skills and default plugins are disabled in an isolated HOME.
  Read/grep/glob and bash archive export execute upstream code, not Python
  implementations of their search behavior.

Codex was compiled with the available Rust 1.97.0, versus upstream's 1.95.0
pin; source commit and binary hash are recorded. Native tools are tested on
this macOS host. This is not a claim of byte-identical official release builds.

## Data boundary and experimental permission envelope

Each arm/case has a separate HOME/store. Only that cutoff's records are loaded.
No questions, answers, other cases or current repository data are placed inside
tool-readable roots. Fixture IDs/timestamps are synthetic metadata; only
original message content counts as historical evidence. Historical workspace
snapshots and pre-existing output spools are not invented. OpenCode output files
created by the retrieval tools during the exam are real native spool files.

A shared study guard admits read-only rg/grep, jq, cat/ls/head/tail/wc and the
specific native archive command. It rejects expansion, scripts, other paths,
permission escalation, other sessions and arbitrary writes. Allowed readers'
results are never rewritten or simulated. Scope denials are labelled separately
from native errors. Original native tools enforce additional native policies.
Codex allows queries only in the workspace/rollout directory; OpenCode only in
the workspace/native tool-output directory (not current-query logs).

The one scratch-write exception is `opencode export SID > retrieved/archive.json`.
A preflight exposed large native CLI JSON being cut short when stdout is a pipe;
exporting to a regular file is a native supported usage that preserves it. The
same applies to diagnostic-driver stdout framing: capture to a regular file,
then parse it, without modifying the returned native object. Do not patch the
upstream CLI or pretend truncated pipe JSON is a complete export. Subsequent
native read/grep or jq/rg read that scratch export. Read-only pipes and `&&`
sequences of admitted operations are allowed; arbitrary redirection is not.

## Exam budget and guidance

- At most 12 admitted tool requests, with additional requests denied; count
  model turns, requested tools, actually executed native tools, native errors
  and scope denials separately.
- Stop new retrieval after 256 KiB UTF-8 output is delivered. Do NOT clip a
  native result to enforce this: the final result may cross the threshold.
- Request at most 10000 native output tokens and no interactive TTY. Native
  read defaults/line clipping and OpenCode's 50 KiB/2000-line mechanisms remain.
- Give every arm the selected archive ID/address and documented native command
  hints. This measures recovery when an entrypoint is known, not whether the
  model independently discovers the product's storage layout.
- Same question, exact closed-projection hash, common read-only task instruction;
  one draw per condition. No new compactions, alternate models, retries chosen
  for better scores, or population significance claims.

## Preflight acceptance (no paid calls)

1. Native Codex parser accepts all fixture records; native rg retrieves a
   sentinel; the native sandbox rejects writes and an outside sentinel read.
2. Native OpenCode import/export preserves message parts, large exports survive
   regular-file output, native shell truncation writes the full captured output,
   and native jq/rg can recover a value near its end. Native read/write denial
   tests and study-scope tests pass.
3. Native Future history CLI retrieves a sentinel from its real database and
   refuses the wrong session scope.

Run with `NATIVE_RETRIEVAL_TEST_ROOT=~/compact-exp/native-open`:
`python3 -m unittest discover -s scripts/abc_experiment -p test_native_retrieval.py -v`.
Without that explicit native toolchain, those integration tests skip, not pass.

## Evidence and stopping

Freeze binary/code hashes, native source commits, corpus/projection/question
references and opening spend before any paid call. Persist every native result
and model request/response. A partial case with model calls cannot silently
restart against newly generated native tool outputs. Native startup/import or
transport failures halt for investigation; do not call incomplete exports,
empty schemas or unavailable services successful retrieval.

Only after all 72 cases finish and the records are independently checked may
this phase be called complete. Report results as local/API native-mechanism
replay under these permissions, not a universal product ranking or a test of
Codex's subscription-gated hosted history service.
