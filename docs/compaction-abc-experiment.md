# Compaction strategy comparison (A/B/C/M/Codex/OpenCode)

**Question:** when S2 already protects original user/assistant text, is a paid
model summary the right way to spend the compaction budget — and how do other
agents' published strategies compare?

The runtime default is C. See [C compaction](compaction.md); this page records the
experiment behind that change.

## Strategies compared

| Arm | What survives compaction | Source |
|---|---|---|
| **A** | protected originals + recursive model summary + recent tail (earlier default) | this repo, pre-C |
| **B** | protected originals + tail, summary deleted and not replaced | ablation of A |
| **C** | protected originals + fixed 2K deterministic tool-evidence index + tail | this repo, current default |
| **M** | summary + recent tail; covered originals dropped | `origin/main` at experiment time |
| **Codex** | **all user messages (≤20 000 tokens) + summary**; no assistant text, no tool output | `openai/codex` @ `b13164d8`, `compact.rs::build_compacted_history` + `templates/compact/prompt.md` |
| **OpenCode** | **summary + retained tail** (`min(15 000, max(2 000, usable/4))`), tool output truncated to 2 000 chars when summarized | `anomalyco/opencode` @ `e03db9bc`, `session/compaction.ts` + `session/message-v2.ts` |

Codex and OpenCode are reimplementations of the selection rules read from those
commits, not forks; the prompts were transcribed verbatim and every file's git
blob SHA is recorded in [abc_external_provenance.json](../scripts/abc_external_provenance.json).
Not modelled: Codex remote-v2 compaction (server-side, needs an OpenAI-hosted
provider) and its `Feature::TokenBudget` path (fresh context window, no summary);
OpenCode's `compaction.prune` (disabled unless configured).

## Results

Closed-book, 48 fields per stage (2 chains × 2 models × 12 fields):

| Arm | stage 1 | stage 4 | stage 8 | total | mean context |
|---|---:|---:|---:|---:|---:|
| A | 41/48 | 43/48 | 42/48 | 126/144 | 14.1 K |
| B | 21/48 | 26/48 | 26/48 | 73/144 | 12.7 K |
| C | 42/48 | 44/48 | 44/48 | **130/144** | 14.1 K |
| M (`origin/main`) | 43/48 | 38/48 | 36/48 | 117/144 | 5.4 K |
| **Codex** | **46/48** | **47/48** | **47/48** | **140/144** | **1.6 K** |
| OpenCode | 43/48 | 44/48 | 33/48 | 120/144 | 5.2 K |

Context sizes are harness estimates, not provider tokens (see limits).

Four findings:

1. **Codex wins on both accuracy and context size.** It reads the entire history
   when summarizing and keeps every user message, so its projection is ~1.6 K
   tokens yet it answers 140/144. Nothing else comes close on this fixture.
2. **Summary-only retention degrades across repeated compactions; C does not.**
   M is the best of A/B/C/M at the first compaction (43/48) and falls to 38 and 36
   as each summary must carry more. C stays flat (42/44/44) because its evidence
   is rebuilt from the journal on every round instead of being re-summarized.
   OpenCode shows the same decay, sharply: 43 → 44 → **33**.
3. **Deleting the summary is still the worst option (B, 73/144).** Its losses are
   tool facts that never appeared in user or assistant prose.
4. **C is the cheapest that holds up.** It costs nothing per compaction and stays
   within 10 fields of Codex over 144.

## Cost

| Group | Requests | CNY | Per compaction |
|---|---:|---:|---:|
| A/B/C | 200 | 3.49 | A ¥0.037, B/C ¥0 |
| M | 48 | 0.79 | ¥0.018 |
| Codex | 33 | **10.49** | **¥0.42** (9.07 M input tokens) |
| OpenCode | 42 | 0.83 | ¥0.030 |
| **total** | **323** | **15.60** | |

Codex is the most accurate and the most expensive: its summarizer reads the whole
live history, so one compaction costs ~23× a summary-only compaction and ~3× the
entire A/B/C arm. Note the asymmetry: the journal-backed arms (A/B/C/M) rebuild
their projection from the archive for free, while the destructive arms pay for
every intermediate compaction they need to keep their history inside the window.

## How the external arms were run

Codex and OpenCode *replace* the conversation, so the Nth compaction reads the
already-compacted history plus what was added since — not the raw archive that
journal-backed arms legitimately rebuild from. Two compactions therefore exist:

* **probe stages force one** (stages 1, 4, 8), so the recorded projection is
  comparable with the other arms at the same boundary — that is the ablation;
* **between probes the agent compacts on its own overflow trigger**, before the
  accumulated history would exceed the provider window. Without this the arm would
  carry a history no real session can hold: the first attempt at a forced
  stage-4 compaction was rejected with
  *"maximum context length is 1048576 tokens. However, you requested 1072531"*.

That rejection also calibrated the harness: the provider counted 1 064 339 tokens
for messages the harness estimated at ~766 K (~1.45×). The overflow threshold uses
that measured factor. Each arm keeps its own output budget: OpenCode hard-codes
4 096, Codex uses the model maximum (capped at 65 536 here).

## Limits

- Codex and OpenCode are single-point-of-source reimplementations. Their retention
  rules and prompts were read from specific commits; a later upstream change would
  invalidate the numbers.
- The probe stages are forced for every arm. Between probes the external arms use
  their own trigger, so their boundary state can be the result of an overflow
  compaction rather than a boundary one; the recorded `compactions` field in each
  projection says which happened.
- Harness token estimates are ~1.45× lower than the provider's count on this
  fixture, so the context column understates real prompt sizes.
- OpenCode hit its own 4 096-token summary ceiling on GLM twice (truncated
  summaries are accepted, as its code does) and once returned an empty summary,
  which OpenCode treats as *declining* to compact — recorded as `declined`, not
  rewritten into a fake summary. That is a real fragility of running a 4 K output
  cap against a large history.
- This is fact recall, not task continuation: it does not measure whether an agent
  can resume the work.
- Two synthetic chains from one blueprint with structured tool records; not two
  independent natural task types, and the questionnaire rewards carrying tool
  evidence.
- The ledger keeps every failure: 4 invalidated early OpenCode calls (wrong tail
  budget), one interrupted M request that was retried, and Codex attempts rejected
  for context length before the trigger was calibrated.

## Reproducing

See [scripts/abc_experiment/README.md](../scripts/abc_experiment/README.md). Raw
data, fixtures, ledger and scoring stay in the local, git-ignored
`.future/research/abc-summary-a47313/`.
