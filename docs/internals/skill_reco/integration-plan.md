# Plan: optional "smart skill recommendation" (desktop / TUI / mobile)

> **Historical document — the plan is implemented; kept as a record of the design decisions taken at
> the time.** How the delivery differed: it landed on one branch (`feat/skill-reco-ui`) rather than
> the five PRs suggested here; recommendation went into the agent as a single RPC (option A below);
> the trigger decision lives on the client (desktop / TUI / mobile each implement it, and each keeps
> its own daily budget); the toggle defaults to **on** (decided later in the PRD, which is what this
> document calls "default off"); the length gate became **≥30 bytes** (ten Chinese characters); the
> timeout became **3 s** (it was 1.5 s, below Jev's measured p95 of ≈1.4 s, so the tail of the
> distribution was discarded and was indistinguishable from "no skill fits"; all three clients now
> lock the input and spin the send button for the wait); and Jev has since moved into the Future provider, which retires the
> "how to configure the temporary key" question (see §Four decisions needed below). **Treat the code
> and the PRD as authoritative**, not the older numbers here. The evaluation data still holds:
> [evaluation.md](evaluation.md). Product rules are governed by `skill-recommend-prd-v1.9` (not kept
> in this repository).

## Two points in this requirement that would be rejected (answered first)

1. **"Only recommend uninstalled skills ⇒ installed ones need not go into the Jev input"** — **correct,
   and stronger than correct.** Do not put installed skills in `state` at all. An installed skill is
   already available, so recommending it wastes a call and may invite the user to "install it twice".
   So Jev's option table = **the subset of the catalogue that is not installed**. That is exactly the
   input construction in §5.
2. **This demo's two-stage shape (stage 1 + stage 2) is not what to copy here.** The requirement asks
   for **one call, ≤1 second, at most one suggestion**, which corresponds to **the demo's stage-1
   single-Choice version** (one call plus a none gate, 0.82 s / 8,765 tok), not the two-stage shape
   with a re-check. Cost and latency should be estimated from that.

## Target shape (assembled from the description; confirm these seven rules)

| # | rule | note |
|---|------|------|
| 1 | a new "smart skill recommendation" toggle in the management UI, **default off** | one per client settings page |
| 2 | triggers only on **a new conversation, the first message, and input length ≥ a threshold** | threshold suggested as `MIN_QUERY_CHARS=6`, adjustable |
| 3 | when it triggers, **intercept the submit** and call Jev with a **1 s** timeout | timeout / no suggestion / error → let the submit through |
| 4 | on a hit, **hold the submit** and show one recommendation card above the input | card holds skill name / description / version + "Install and use" and "Ignore and send" |
| 5 | "Install and use" → install the skill + **append its slash command to the end of the input** → then send | reuses the existing `install_skill` |
| 6 | "Ignore and send" → do not install, send as typed | |
| 7 | **if the user has already selected ≥1 skill in the input, do not trigger** | detect an existing slash command or `@skill` |

## Architectural facts that decide the layer (verified)

- **Catalogue and installation**: desktop and mobile use the platform catalogue
  (`list_available_skills` / `install_skill` / `uninstall_skill` in
  `desktop/src/integrations/skills/skillsClient.ts`, a Tauri command → platform `GET`). The TUI goes
  through the agent's `refresh_skills` / `get_commands`. Installation lands in
  `~/.future/agent/skills/` (`agent/src/skills/mod.rs`).
- **Where the Jev call goes**: recommendation needs "the uninstalled catalogue + a Jev credential +
  one HTTP call". **Putting it in the agent (Rust) as one RPC command is the cheapest way to avoid
  triplicating it across the clients** (see below), at the cost of a new agent dependency and a
  temporary key configuration. **Alternative**: call it from each originating client — desktop and
  mobile through the Tauri side or the remote relay, the TUI through the agent. **Preference: the
  single point in the agent.**
- **"Is this the first message of a new conversation"**: the client knows best (empty input, no
  messages in the thread), so **that decision belongs on the originating side**, not in the agent.
- **The 1 s timeout**: `Promise.race` on the client / `tokio::time::timeout` in the agent, and on
  timeout the submit goes through as "no suggestion". **Recommendation must never be able to block a
  send** — consistent with the demo's stance that the threshold and gate are ours: failure means let
  it through.

## Where the recommendation lives (two candidates; **A is preferred**, your call)

- **A. A new agent RPC command `suggest_skill`** (a typed payload in
  `packages/rpc/proto/future.proto` + an implementation under `agent/src/rpc/commands/*` + the
  `suggest.mjs` logic ported to Rust).
  Pros: one copy of the logic and thresholds for all three clients; the key is configured only in the
  agent; when Jev moves into the future provider, **only the agent changes**.
  Cons: crosses crates (`packages/rpc` is the wire contract, so changing it means testing the direct
  consumers), and it is the largest piece of work.
- **B. Implement it in each originating client** (desktop and mobile reusing the existing TS skill
  client plus their own Jev fetch; the TUI through the agent).
  Pros: fast, and the wire contract is untouched. Cons: thresholds and logic scattered across three
  places, and three places to change when Jev moves into the provider.

## Cost basis (asked for explicitly, to go into the planning cost)

- **Model cost = one Jev call per trigger** (not per conversation or per user). On the demo's measured
  single Choice: **8,765 tok = ¥0.0027 per call** ($0.042/Mtok, output free, USD→CNY at 7.2).
- Trigger frequency is bounded by four gates: the toggle defaults off, new conversations only, the
  length threshold, and no pre-selected skill. Roughly: a user who enables the toggle and reaches the
  length threshold on **N** new conversations per day costs about **N × ¥0.0027/day**.
- **That is the model cost only**; it excludes the platform catalogue and installation themselves
  (unchanged, nothing added). Once Jev is in the future provider this should move onto Future's token
  accounting.

## Breakdown (five PRs suggested, each independently testable)

1. **`feat/skill-reco-agent`**: the agent's `suggest_skill` RPC plus the Jev client (1 s timeout, none
   gate 0.15, uninstalled subset, dev key from an environment variable). **This is the foundation**;
   every other PR depends on it.
2. **`feat/skill-reco-settings`**: the toggle (default off) plus persistence across all three clients.
3. **`feat/skill-reco-desktop`**: first-message interception, the recommendation card, installation,
   appending the slash command, and letting the submit through on timeout.
4. **`feat/skill-reco-mobile`**: the same, as React Native interaction.
5. **`feat/skill-reco-tui`**: the same, in the terminal (simplest form: report the hit, and after
   installation splice the command into the first message).

> Each PR follows the repository rules: sync `origin/main` → test only the crates/modules changed →
> enable auto-merge as soon as the PR opens.

## Testing ("thorough", as asked)

- **agent**: unit tests for `suggest_skill` (building the catalogue subset, the gate threshold,
  timeout means let it through, missing key means let it through or report).
- **desktop / mobile / TUI**: component and interaction tests (intercept → card → install → append →
  send; ignore → send; timeout → send directly; a skill already present → no trigger; toggle off → no
  trigger). Desktop uses the existing `*.test.tsx` with vitest, mobile uses jest, the TUI uses Rust
  tests.
- **Reuse the demo's evaluation** as a regression baseline: after changing a threshold or the logic,
  run `bench/predict-jev.mjs` to confirm the score did not drop.

## Four decisions needed

1. **Recommendation as one agent RPC (A) or in each client first (B)?** A is preferred.
2. **How to configure the Jev key in the meantime**: in something like
   `~/.future/agent/auth.json`, or read only from a `FUTURE_SKILL_RECO_KEY` environment variable? (The
   demo did the latter; before the provider move the latter is preferable, to avoid touching the auth
   structure.) — *Moot: Jev is in the Future provider now, so the account credential is used.*
3. **The length threshold**: 6 (the demo's `MIN_QUERY_CHARS`), or longer (say 20 — more conservative,
   less intrusive)? — *Decided later as ≥30 bytes.*
4. **One client first** (say desktop) end to end, then copy to mobile and the TUI — or the agent
   foundation first and all three in parallel?
