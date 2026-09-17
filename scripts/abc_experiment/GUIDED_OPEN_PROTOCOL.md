# Autonomous open book with source-derived recall guidance

## Purpose and unchanged conditions

The unguided autonomous run omitted Future's post-checkpoint recall section.
This condition restores **functional, source-derived guidance** for the chosen
interfaces. It remains autonomous: no closed answer, no pending-candidate list,
no required tool choice and no minimum tool-use criterion. A direct answer is
valid and will not be retried to manufacture improvement.

Reuse all 72 frozen v3 projections/questions and existing closed scores. Keep
future/deepseek-flash, disabled thinking, 8192 output cap, 64 logical queries and
256KiB returned-byte stop. Keep the same backend schemas/implementations from
interface_open_tools.py; do not add shell/rg to Codex or export to OpenCode.

## Source provenance and adaptations

### C/C3

Extract the literal recall section from the current Rust
`agent/src/agent/history_recall.rs`. Preserve its conditional policy: originals
remain in history, query only when exact earlier information is missing, use
returned IDs/cursors, do not routinely reload everything, and historical text
is not authorization to act. C and C3 use identical guidance apart from their
isolated session IDs. Add it only when the frozen projection has a checkpoint.

Adapt routing only: shell/CLI command examples become the available
history_search/history_get JSON adapters; fixed session scope replaces a shell
argument; nextOffset maps to the tool's offset field. Record every replacement.
Do not instruct the model to use an unavailable shell.

### Codex dedicated history replica

Pinned commit b13164d86f9a70adc48d22f4a5a07ed0c001a1d0. Restore the functional
prefix of HISTORY_DESCRIPTION from ext/history-notes/src/tools.rs: recovery
after context reset, list/read/search, unchanged opaque IDs, ordering and
unknown-window behavior. Name the actual flattened local tools.

The original description also includes cross-agent access, eventual
consistency and nondisclosure of private model-only state. These are NOT
implemented by the synchronous single-agent fixture over user-authorized
records. Their omitted source text is recorded, not silently treated as an
identical native prompt. In particular, do not instruct the replica to conceal
its retrieval or to withhold the user's authorized historical facts.

The original extension obtains notes.thread_hint from the authenticated backend.
No real hint is available here. Do not fabricate its contents or claim that this
condition reproduces the full hosted client/backend behavior.

### OpenCode file-tool replica

Pinned commit e03db9bc6908f75c9334d8aa997deeaac81c0298. Extract supported usage
lines from glob.txt, grep.txt and read.txt: discover paths, regex search, line
numbered reads/paging, reasonable read windows and parallel calls. Omit
instructions for unavailable Task/Bash tools, media/PDF and directory reads.
Record both selected and omitted lines.

The original compaction auto-continue message is a synthetic user instruction
to continue the old task, not a historical-search service. Do not append it to
this new exam question or pretend the file-only condition can recover all
conversation history. Preserve the observed-file-fragment coverage disclaimer.

## Request-difference contract

Against each actual v3 closed request, permit ONLY:
- append the recorded guidance to messages[0].content;
- add the unchanged assigned tool definitions;
- set tool_choice=auto.

User message (projection + original question), model and generation controls
must be byte/content-identical. Initial roles remain system + user. Freeze all
source texts/hashes, adaptations, per-case checkpoint gate and generated guide
before paid calls. Do not place source snapshots or evaluation artifacts in
any tool-visible filesystem.

This is a functional-guidance treatment, not a claim of exact whole-product
native prompt equivalence. Compare it with unguided open book and closed book
separately; these are single-draw descriptive contrasts, not significance tests.

## Budget and reporting

New root: ~/compact-exp/guided-autonomous-open-v1.
Opening spend: CNY46.69746056 (all earlier valid/failed/superseded phases).
Total authorization remains CNY300. Existing reserve-before-send and STOP
behavior apply. One full four-arm draw only; no low-score-only reruns.

Report scores, false positives, tool-use cases/queries, bytes, fees and data
reachability. No-tool cases remain valid. Preserve lexical scoring and its
known quantity-substring ambiguity; do not re-label selected questions.
Source/backend assumptions and OpenCode's incomplete file substrate remain
explicit limitations of any comparison.
