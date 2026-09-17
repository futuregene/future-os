# Historical retrieval: C/C3, Codex and OpenCode

Chinese detailed guide: [compaction-retrieval-mechanisms.zh-CN.md](compaction-retrieval-mechanisms.zh-CN.md).

## Scope and versions

C and C3 use the **same native historical-retrieval backend**. They differ in
whether the compacted projection includes a sticky model summary, not in which
journal they can search. Compaction changes the model's projection; it does not
remove the original journal. Previously deleted data cannot be recovered.

External versions are pinned to Codex `b13164d86f9a70adc48d22f4a5a07ed0c001a1d0`
and OpenCode `e03db9bc6908f75c9334d8aa997deeaac81c0298` (source package 1.18.31).
The Codex condition is third-party API/local-inline, not its subscription-gated
hosted history/notes service. The experiment replays reduced frozen text/tool
records; it does not restore original production workspaces, media, permissions
or every cache.

## C and C3: the complete native path

```text
Missing earlier fact
  -> existing shell tool
  -> tools::shell_tool().handler
  -> run_shell_with_capability / spawn_shell_with_report
  -> sandbox::shell_invocation (complete command, not split argv)
  -> future session history search/get
  -> CLI RpcClient
  -> search_session_history / get_session_history_entry RPC
  -> Manager::search_history / read_history_entry
  -> original entries + message_blocks
  -> native JSON, stdout/stderr and shell exit status
  -> next model request
```

`history_recall.rs` appends request-only recall guidance with the current session
ID. `run_loop.rs` and `rpc/session_prompt.rs` gate it on a valid checkpoint,
persistent/eligible session, permissions and shell availability. It is not a new
persistent chat message. The production instruction says to query **when exact
old information is missing**, not routinely reload the entire database. This
is different from an experimental mandatory-retrieval condition.

History RPCs are handled before model-runtime creation. They read the specified
persisted session without a summary call or a compaction/run lease. The local
query itself does not call an LLM; subsequent model requests containing its
results still incur ordinary model charges.

### Search

```sh
future session history search --session SESSION_ID --query "exact value" --limit 5 --json
```

- Searches original user/assistant text, tool argument JSON and tool results.
- Excludes reasoning, hidden provider metadata, checkpoints and lifecycle rows.
- Literal substring matching, ASCII case insensitive; `a|b` is not an OR query.
  Exact tool-call IDs can also match.
- Nonempty query, at most 200 characters, no NUL.
- Default 5 matches, allowed 1–20; newest entry positions first, then block order.
- Returns entry ID, block identity/kind, bounded snippet, byte offset and hasMore.
  Unshown text is not evidence of absence.

### Get

```sh
future session history get --session SESSION_ID --entry ENTRY_ID --offset 0 --limit 8192 --json
```

- Reads an original entry belonging to that session.
- Offset addresses concatenated readable UTF-8 block bytes, not lines or tokens.
- Default 8192 bytes; allowed 4–32768. Start offsets must be character boundaries.
- Returns text chunks with block/tool identity, hasMore and nextOffset.
- Search's byteOffset can be passed to get; continue using nextOffset. Do not
  repeat the historical tool just to recover its result.

### Composition is native shell behavior

```sh
future session history search --session S --query A --json;
future session history search --session S --query B --json

future session history search --session S --query A --json |
  jq -r '.matches[].entryId'

for term in A B; do
  future session history search --session S --query "$term" --json
done
```

No new batch-search API is needed. Unix passes the complete command to the
selected shell's `-c`; Windows production code uses its PowerShell wrapper and
encoding. The native shell defaults to 120 seconds, merges stdout/stderr,
retains at most the last 500000 output bytes, and appends an exit footer. A
compound command's final zero status does not prove every earlier command
succeeded.

Relevant native Rust: `agent/src/agent/history_recall.rs`,
`agent/src/agent/run_loop.rs`, `agent/src/rpc/session_prompt.rs`,
`agent/src/tools/mod.rs`, `agent/src/sandbox/mod.rs`,
`cli/src/commands/session_history.rs`, `cli/src/rpc.rs`,
`agent/src/rpc/commands/mod.rs`, `agent/src/session/history_query.rs`.

## Codex: local and hosted modes are not interchangeable

```text
Original conversation -> local rollout
Model -> native exec_command -> native argument/permission/sandbox handling
      -> UnifiedExecProcessManager -> shell and permitted readers
      -> native output/exit status, or an active exec session ID
      -> write_stdin for additional output -> answer
```

In this local/API condition, a fabricated local `history.search_contents`
endpoint would add a capability not provided by the selected configuration.
The replay validates rollout records with the original Rust
`codex_rollout::parse_rollout_line`, runs the original CLI, obtains its real tool
definitions, and invokes the original ExecCommandHandler/UnifiedExec machinery.
The native exec process stays alive through an exam so polling session IDs are
real. The default exec output budget in this version is 10000 tokens; one exec
can combine multiple searches and filters.

A local Responses event adapter only forwards the examiner's requested tool
call into the original CLI and receives its native result. It contains no
search implementation, does not manufacture retrieval results and does not
answer the exam.

The separate history/notes extension requires appropriate provider, backend
authentication and token-budget configuration. The experimental-context path
also checks model support, ContextManagement and eligible ChatGPT subscription
conditions. This third-party DeepSeek condition cannot silently substitute a
local emulator for that hosted service.

Pinned source:
- [ExecCommandHandler](https://github.com/openai/codex/blob/b13164d86f9a70adc48d22f4a5a07ed0c001a1d0/codex-rs/core/src/tools/handlers/unified_exec/exec_command.rs)
- [Tool schemas](https://github.com/openai/codex/blob/b13164d86f9a70adc48d22f4a5a07ed0c001a1d0/codex-rs/core/src/tools/handlers/shell_spec.rs)
- [Rollout parser](https://github.com/openai/codex/blob/b13164d86f9a70adc48d22f4a5a07ed0c001a1d0/codex-rs/rollout/src/lib.rs)
- [Hosted-mode eligibility](https://github.com/openai/codex/blob/b13164d86f9a70adc48d22f4a5a07ed0c001a1d0/codex-rs/core/src/session/token_budget.rs)

## OpenCode: files, session export and full-output caches

```text
Model -> native bash/read/grep/glob
      -> permitted files, or opencode export SESSION_ID
      -> Session.Service.get/messages
      -> native session/message/part store -> JSON
      -> native output cache if applicable -> readers -> answer
```

OpenCode is not limited to the current working tree. Its native CLI can export
stored session messages; that is not merely the currently compacted projection.
The optional sanitize flag is not default transcript deletion.

- glob performs native file discovery with a 100-result bound.
- grep uses the native Ripgrep service and regular expressions, returning file,
  line and text information, with the tool's current 100-match bound.
- read uses 1-based line offsets, defaults to 2000 lines, and has its own 50 KiB
  and long-line limits; it is not Future's byte-oriented get.
- bash uses the native shell/parser, permission checks, execution, timeout and
  output machinery, including calling the native export command.
- Applicable long-output paths retain complete output in files and return
  outputPath hints. Defaults include 2000 lines/50 KiB and seven-day cleanup.
  Individual tools may handle truncation themselves. Absent, cleared or expired
  content cannot be recovered by inventing a cache.

The replay calls original import/export and checks a complete parts roundtrip.
The original `/experimental/tool` endpoint provides actual schemas.
`debug agent build --tool ...` enters the original registry and tool.execute;
its result.output goes to the examiner unchanged, not through a Python search
implementation. This debug entrypoint's handling of ask differs from a full
interactive client; explicit allow/deny is used, and no full approval-flow
identity is claimed. Runtime is Bun 1.3.14.

Black-box testing found that large native CLI JSON may not drain completely to
a pipe before exit. Exporting to a regular temporary file before reading it is
supported native usage, not a patch to the product or permission to accept an
incomplete export as complete.

Pinned source:
- [Registry](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/registry.ts)
- [Native debug execution](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/cli/cmd/debug/agent.handler.ts)
- [Export](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/cli/cmd/export.ts)
- [Read](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/read.ts), [grep](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/grep.ts), [glob](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/glob.ts)
- [Shell](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/shell.ts), [truncation/cache](https://github.com/anomalyco/opencode/blob/e03db9bc6908f75c9334d8aa997deeaac81c0298/packages/opencode/src/tool/truncate.ts)

## C/C3 adapter correction and remaining limits

The old experiment misleadingly named an executor shell while doing
`shlex.split(command)` followed by exactly one future process. It interpreted
`--json;` or `--limit 5;` as arguments. All 13 nonzero C3 CLI exits in the audited
run were compound commands. Codex was allowed real shell composition.

The new path is `native_open_exam.py -> NativeFutureShell.execute_shell ->
abc_future_shell_probe -> production shell_tool().handler -> native scope and
shell_invocation -> full shell command -> native CLI/RPC/journal`.
It also exports the original Rust tool definition. A per-session launcher checks
the already-parsed argv of each CLI invocation and forwards it; it does not
parse shell syntax or implement history search.

Local tests verify semicolon batching, jq pipes, loops, search/get byte offsets,
nonzero statuses, another-session refusal, explicitly denied sibling reads and
workspace write denial. No paid model exam was run for this correction.

**Do not overstate safety:** the adapter preserves native Future path rules and
adds explicit denials, not a global read allowlist. Native Future networking is
not disabled by default. Full-shell data isolation and comparison permissions
still need review before a new paid run; the entrypoint requires an additional
operator acknowledgment. Restoring native execution is not a claim that every
fairness requirement has been met.

Experiment glue lives in `agent/examples/abc_future_shell_probe.rs` and
`scripts/abc_experiment/native_future_shell.py`, `native_codex.py`,
`native_opencode.py`, and `native_stores.py`. The latter's old argv-only executor
is retained solely for historical replay. Algorithms and actual tool execution
must remain the products' own code; glue may isolate, convert inputs, invoke
and record, not silently remove or invent capabilities.
