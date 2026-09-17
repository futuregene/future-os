# Interface-level open book (user-approved new plan)

## Arms and scope

Reuse v3's exact closed-book projections, questions and covered record ranges.
Do not reuse scores from either native-shell open run.

- **C and C3:** identical `history_search`/`history_get` thin tools. Each tool
  invokes the actual Future CLI and persisted Agent database. No fake shell,
  no shell parsing, no model call inside the query. Require `legacy_imports =
  imported` and an exact normalized-message count before the first exam call.
- **Codex:** LOCAL reproduction of the dedicated `history.*` interface, not
  exec_command/rg and not a claim to run the subscription-gated hosted backend.
  Window boundaries use the recorded compaction schedule. Window/item pairing,
  case-sensitive literal search, role/tool filters and character offsets are
  tested. Missing namespace metadata remains unknown. Local response shapes,
  Python Unicode-character offsets, 4000-character head excerpts/defaults and
  64KiB complete-JSON response budget are explicit assumptions; they are not
  observations of the closed-source server.
- **OpenCode:** LOCAL glob/grep/read file-tool reproduction. Regex and glob
  selection use ripgrep over an isolated materialization, never the host's
  current project. Preserve full directory paths. Synthetic files use known
  generator contents. Real files are last path-associated read observations,
  which can be fragments with missing original offsets. A subsequent write/edit
  attempt invalidates the observation until another read. Do not invent a
  combined tool-output.log for unlocated output, or call these fragments a
  complete historical working tree. No CLI export or conversation API is added
  to this arm.

These arms have different information substrates. Report both OVERALL recall
and recall within the mechanically reachable union of projection + lookup
substrate. An inaccessible fact is a coverage limit, not proof the file search
implementation is inferior. Correct answers outside exact containment are
recorded separately; substring containment is not full semantic reachability.

## Common exam conditions

- Same `future/deepseek-flash`, thinking disabled, 8192 output tokens.
- Use already established projection facts directly; query unresolved facts
  where possible. Do not force C/C3 to reload everything. Tool choice is auto,
  and cases with no lookup are reported, not called failed retrieval.
- 24 logical tool requests per case; multiple calls in one model turn count
  separately. Model turns, queries, errors, pagination and bytes are different
  measures. No arbitrary shell batching is counted as one query.
- Stop initiating queries after 128KiB returned UTF-8 bytes. Do not corrupt a
  native Future JSON response by cutting it mid-object. Local Codex responses
  have a documented complete-JSON cap; file read/grep outputs have documented
  local bounds. Final output can cross the aggregate stop threshold by one
  response; this is not advertised as a hard per-probe byte ceiling.
- One draw per condition, arm order randomized within chain/boundary with the
  frozen seed. No significance claim from repeated boundaries or 178 fields;
  only three independent real-session sources.
- User authorization is CNY300 total including ALL previous phases. Opening
  spend is CNY43.77160008, linked to the previous immutable ledger/report.
  Reserve before sending; preserve unknown charges and failures. No silent
  retries of partial cases, no model substitution.

## Validation and provenance

`test_interface_open.py` covers window-scoped reads, Unicode offsets, literal
case sensitivity, tool filters, future-record exclusion, directory collisions,
write invalidation, traversal denial and complete JSON bounds. Native Future
CLI/database behavior is tested independently and each actual case verifies
its import state and message count before calling the examiner.

Freeze source/binary/schema hashes and file visibility before paid calls.
Preserve every request, response, tool trace and score. Do not overwrite older
runs. A final report must explicitly identify replicas and substrate gaps; it
is not a complete native-product ranking.

## Invocation

```sh
python3 -m unittest discover -s scripts/abc_experiment -p test_interface_open.py -v
python3 scripts/abc_experiment/interface_open_exam.py \
  --closed ~/compact-exp/rerun-fidelity-v3 \
  --prior ~/compact-exp/native-open/run-v2 \
  --output ~/compact-exp/interface-open-v1 \
  --future target/fidelity/debug/future \
  --dumper target/fidelity/debug/examples/abc_c3_probe \
  --bridge target/fidelity/debug/examples/abc_probe_bridge --prepare-only
# Remove --prepare-only to execute. --detach starts one monitored process.
```
