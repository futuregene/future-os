**提案，未立项**

# Borrowing from `deepseek-ai/dsh-superagent-prototype`: a one-page proposal

Read: <https://github.com/deepseek-ai/dsh-superagent-prototype> (MIT, ~8.4k lines TypeScript, ~2k lines of numbered design docs, ~90 test files). It is a resident agent for **one** repository: it listens to GitHub/Feishu, decides whether a message authorizes work, runs that work in its own worker session and directory, and answers in the original conversation.

This page borrows **design and test shapes only**, never its runtime assumptions, and never its code text; every claim points at `file:line` on both sides. Implementing nothing yet — pick one item below or reject all four.

## 0. Why only a little

Its scarce assets are three: an authorization model, an attention model, and prompt engineering with a contract test. Only one of the three can be adopted here without a product decision. Everything else it does, we already do at equal or greater depth — the difference is coverage, not concept, and section 3 names where.

Its own runtime premises do not hold for us, and section 2 lists them as anti-patterns for this repo.

## 1. What to borrow

### B1. A contract test over everything the model can read — recommended first

- **Theirs**: `tests/model-surface.spec.ts:40` (per-role character budget: 59k orchestrator / 40.5k worker), `:50` (retired vocabulary list), `:53` (banned-sentence list), `:377` (a word limit the prompt *states* is asserted against the constant), `:389-396` (the budget assert). The test reads the assembled prompt, the tool definitions, the skills and the notice texts, and fails on any tool name, schema option, environment variable or glossary word that does not exist.
- **Ours**: the prompt is assembled at `agent/src/prompt/mod.rs:19`, from ~1,400 lines of prose, 4 built-in tools (`agent/src/tools/mod.rs:482`), skills discovered from user directories (`agent/src/skills/mod.rs:31,69`) and three compiled assets (`agent/src/prompt/communication.md:1-12`, etc.).
- **Gap**: nothing checks that text against reality. The prose names concrete commands and numbers — `future session abort` at `agent/src/prompt/mod.rs:501`, `max 600s` at `:520`, "at most three in flight" asserted by hand at `:1176` — so the CLI can rename a subcommand and the prompt keeps describing the old one until a model hits it.
- **Cost**: one new test file in `future-agent`, ~250-350 lines of assertions, no schema, **no change to existing behaviour**. The only risk is that it is red on day one, so it starts with an explicit, non-empty-known-drift list that may only shrink.
- **Acceptance**: `cargo test -p future-agent --test model_surface` green on today's tree; falsified once by renaming one command in the prompt to a non-existent subcommand and watching it fail. Observable behaviour: none changes — the proof is the failing test when the drift is reintroduced.
- **Rollback**: delete the test file and its list. Zero runtime surface.

### B2. An `observe` mode for autonomous work, and a record of what authorized it

- **Theirs**: `src/policy.ts:84` (`mode: 'off' | 'observe' | 'auto'`), `:191` (the deterministic gate: who may originate work, with strength `explicit`/`inferred`), `:213` (a mandate is revalidated before an ended unit of work resumes), and `docs/002-authorization.md:91-109` (observe records "would accept" and accepts nothing; every mandate records the policy version; a policy change **notifies** live work instead of cancelling it).
- **Ours**: `channels/src/policy.rs:75-120` decides only whether a message is *answerable* (dm/group allowlist, mention gate). It does not carry *what authorized this work* or *under which rules*. Scheduled tasks are explicitly full-permission: `docs/architecture/tasks.md:3`, `orchestration/tasks/src/lib.rs:1`. Goals have gates, but a gate records a decision, not a mandate (`docs/architecture/goal-control-plane.md:55`). we do have `--dry-run`, but only as a preview of destructive commands (`cli/src/commands/session_prune.rs:45`) and the goal backfill replayer (`orchestration/goal/src/console.rs:608`); there is no run mode that records work without acting.
- **Gap**: anyone in an allowlisted conversation can make the agent do work, and there is no way to watch what the agent *would* have done before letting it act.
- **Cost**: three crates (`future-channel`, `future-tasks`, `future-goal`) plus persisted fields, ~600-900 lines. **Changes existing behaviour and changes schema** (task run record, goal event). Must land in three steps: record-only mode → act → revalidate-on-resume.
- **Acceptance**: (1) with observe on, a scheduled task run produces one `would_run` record and **zero** tool calls, asserted from the task ledger; (2) removing a sender from an allowlist makes their previously authorized long-running work refuse to resume — new test `mandate_revalidation`; (3) editing a rule notifies the running work instead of cancelling it, asserted on the event stream.
- **Rollback**: the mode defaults to `off` and the new fields stay nullable; a revert is deleting the read path, not the columns.

### B3. Per-section prompt overrides that remember their built-in base

- **Theirs**: `src/app/prompts.ts:29-30` (each override stores `base`, the key of the built-in text it was edited from), `:186-192` (`defaultChanged` reports an override whose default moved), `:202` (a save names its base revision and is refused when stale).
- **Ours**: `agent/src/prompt/mod.rs:23-24` (`custom_prompt` replaces the identity section wholesale) and `:106-107` (`append_prompt` is appended late). There is no per-section override and no way to tell a user "the default changed since you edited this".
- **Gap**: the moment a user customizes the prompt, every later improvement either silently wins or silently loses.
- **Cost**: `future-agent` settings schema + prompt assembly, ~300-450 lines. **Changes existing behaviour**: the precedence of `custom_prompt`/`append_prompt` against a new overlay has to be defined, and migration must keep today's output byte-identical.
- **Acceptance**: unit tests in the prompt module — an unedited section follows the new default, an edited one keeps the user's text and reports `default_changed`; plus a golden snapshot proving a legacy `custom_prompt` configuration assembles exactly what `build_prompt` produces today.
- **Rollback**: absent overlay = today's behaviour; the field can be dropped without touching the assembly order.

### B4. One stable code vocabulary for "why this ended / why it waits"

- **Theirs**: `src/core/reasons.ts:5-32` — a reason is `{code, values, triggerId}`; agents read the English template, the panel words `reason.<code>` in its own language; sources and gates may add their own codes.
- **Ours**: `desktop/src/features/vibe/attention.ts:23,36` uses a string union for conversations, `orchestration/goal/src/work_items/attention.rs:33-40` uses `status: String` for goals, and the wording lives in `tui/src/workbench/i18n.rs` (1,811 lines) and `desktop/src/i18n/locales` separately.
- **Gap**: "why did this stop, what is it waiting for" has no machine-readable form, so three surfaces word the same fact three ways and cannot be compared or filtered.
- **Cost**: `future-goal` + `future-tasks` code tables and the three client mappings, ~150 lines, no schema (codes ride in existing string fields). Does not change behaviour: old text is kept, codes are added.
- **Acceptance**: `reason_codes_are_stable` (serialized snapshot of the code set) plus a key-existence test per client locale; behaviour unchanged is proven by the existing snapshot tests.
- **Rollback**: nothing reads the new field until a client asks for it; deleting the table falls back to today's strings.

## 2. What not to borrow

| Don't borrow | Their evidence | How copying it breaks us |
| --- | --- | --- |
| One machine, one process, one SQLite file | `src/app/store.ts:54` (opens the file, takes a directory lock) | We run a host, remote peers and a desktop at once; a single-writer-file assumption collides with remote-host and desktop coexistence |
| Executing without approval | `docs/003-execution.md:27,80` — workers run commands as the operator's OS user, and the wrappers "are not a security boundary" | Our sandbox and approval path are a product differentiator; adopting this downgrades approval to prompt discipline |
| One repository per deployment | `docs/007-architecture.md:221`, `src/deployment.ts:51` (`SETUP_FIELDS`) | Our agent works across projects and workspaces; a deployment-wide single repo is a config regression |
| Router/worker as two sessions | `docs/003-execution.md:27-31`, `docs/007-architecture.md:509` (5-second shell and job-wait limits) | Doubles prompt budget and latency; we already have sub-sessions and would gain only politeness rules, which are quoted separately |
| The attention-derivation *pattern* | `src/core/stage.ts:9` | We already have it twice, closer to our model: `desktop/src/features/vibe/attention.ts:36` (three states, first match wins) and `orchestration/goal/src/work_items/attention.rs:103` (goal gates). The real gap is coverage across tasks/mobile, not the idea — see section 4 |
| A durable effect/outbox queue | `src/app/effects.ts:5` (`Later`: defer without counting a failure) | We already have queue + backoff + permanent-failure classification in `channels/src/outbox.rs:122` and `channels/src/delivery.rs:65`; a second implementation forks delivery semantics |
| Never reclaiming records | `docs/007-architecture.md:484` ("Nothing deletes old records yet") | We have already paid for unbounded `run_events` growth (`docs/guide/run-journal.md`) |
| Deployment policies as architecture | 200-line auto-merge ceiling, 100-word chat cap, `docs/003-execution.md:27` job limits | These are one team's rules, not invariants of an agent system |

Its communication prompt is the one prose asset worth quoting (short, attributed, adapt rather than copy): say something only when a person must act or can use a result; a pull request existing is not a delivery milestone; before sending, ask of each clause whether deleting it would stop the reader from understanding or acting; CI and bots can never authorize work; when intent is unclear, stay quiet and let later messages settle it rather than asking a human to confirm chatter.

## 3. Deferred (small, do them when the file is next touched)

| Item | Their point | Our landing spot | Acceptance |
| --- | --- | --- | --- |
| In-memory repository mirror for domain tests | `src/core/memory.ts:10`, 50 lines, same ordering and nested rollback as the SQLite store | Goal/domain tests still need a database file, and our memory notes several flakiness sources from that | The same domain suite passes on the in-memory implementation, including ordering and rollback |
| Single-writer health entries | `src/infra/health.ts:18` (a second claim throws) | Nothing asserts which component owns a health key | Claiming one key twice panics, asserted by a test |
| An architecture inventory that is a test | `tests/docs.spec.ts:58` (every module in `src` must appear in the architecture table) | `docs/README.md` indexes documents, but a new module can land unlisted | Adding a module without listing it fails the docs gate |

## 4. Cost summary (behaviour changes listed separately)

| Item | Crate | Size (estimate) | Schema | Changes existing behaviour |
| --- | --- | --- | --- | --- |
| B1 contract test | `future-agent` | 250-350 test lines | no | no |
| B2 observe + mandate | `future-channel`/`future-tasks`/`future-goal` | 600-900 | **yes** | **yes** (three steps) |
| B3 prompt overlay | `future-agent` | 300-450 | settings schema | **yes** (precedence + migration) |
| B4 reason codes | `future-goal`/`future-tasks`/clients | ~150 | no | no |

## 5. If only one is done

**B1.** It is the only item that improves a running system without changing what it does: the drift it catches is already real (a prompt that names CLI subcommands and numbers, edited by hand, checked by nothing), the blast radius of a revert is one file, and it makes the other three cheaper to review, because the same test file becomes the place where B2-B4 declare what the model is now allowed to read.

B2 has the largest product upside, but it is not mine to start: it requires deciding who may make FutureOS do work, which is a product decision.

## 6. Decisions needed from the owner

1. **B2's precondition**: who may originate work in FutureOS — only the owner, an allowlisted conversation, or a rule list? Without that answer B2 cannot be designed, only sketched.
2. **B3's precondition**: is user-editable agent behaviour a product direction? If the answer is "skills and AGENTS.md are enough", B3 should be rejected rather than deferred.
3. **B4's precondition**: do we want one shared vocabulary across TUI/desktop/mobile, or is per-surface wording acceptable? Today's duplication is deliberate in places (mobile occupies one line), so this may be a "no".

Source material: <https://github.com/deepseek-ai/dsh-superagent-prototype> — `docs/001-domain.md` … `docs/007-architecture.md`, `src/core/`, `src/policy.ts`, `tests/model-surface.spec.ts`. MIT licence; this proposal restates designs and cites line numbers rather than copying code.
