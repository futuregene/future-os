import type { AgentMessage } from "@future-os/thread-projection";
import type { StoredRun, StoredRunEvent } from "../../../integrations/storage/threadStore";
import { buildAssistantRunProjection, matchesSettledRun } from "@future-os/thread-projection";
import { listRunEventsBulk, storedTimeToIso } from "../../../integrations/storage/threadStore";
import { buildAgentFailureContent, buildAgentFailureTitle, userStoppedNotice } from "../agentMessageFormatters";
import { isCompactionDivider } from "../messages/compactionMarkers";
import { runDurationMs } from "../runtime/runProjectionUtils";

/**
 * Backfill run-derived status onto messages projected from agent session
 * entries. Agent JSONL only records message content, not a run's GUI-side
 * outcome (failed/cancelled/model) — that lives in the SQLite `runs` table. So a
 * reload from the agent path would otherwise show every exchange as "complete",
 * losing the Retry/Continue affordance, the "stopped" marker, and the model
 * badge.
 *
 * New Agent entries carry canonical run ids and bind exactly. The positional /
 * timestamp logic below is retained only as a compatibility reader for legacy
 * JSONL written before run identity was persisted.
 */
export function applyRunMetadata(messages: AgentMessage[], runs: StoredRun[]): AgentMessage[] {
  if (!runs.length)
    return messages;
  const runsById = new Map(runs.map(run => [run.id, run]));
  const patched = [...messages];
  const boundRunIds = new Set<string>();

  for (let index = 0; index < patched.length; index++) {
    const message = patched[index]!;
    if (message.role !== "assistant" || !message.runId)
      continue;
    const run = runsById.get(message.runId);
    if (!run)
      continue;
    if (!matchesSettledRun(run.status)) {
      // The agent's save_callback stamps run_id on EVERY persisted assistant
      // message, including the partial snapshots it saves mid-run. Binding one
      // of those — or even leaving the stamp in place — makes the live-bubble
      // guards (streamingBubbleBase's runId check) treat the run as settled,
      // so after ANY mid-run reload (thread switch, reattach, remote activity)
      // the streaming bubble is suppressed and the user sees a frozen
      // "complete" partial with no stop button. Strip the stamp so the entry
      // re-enters the legacy in-flight handling below (dropped when a bubble is
      // alive, rendered plainly otherwise). Self-heals on settle: the on-disk
      // entry still carries meta.run_id, so the next settled reload re-binds it
      // through the canonical path.
      patched[index] = { ...message, runId: undefined };
      continue;
    }
    patched[index] = applyRunToMessage(message, run);
    boundRunIds.add(run.id);
  }

  // Indices of real assistant replies, oldest-first; reversed to newest-first to
  // zip against newest-first legacy runs. Canonically-bound exchanges never enter
  // this fallback.
  const turnIndices = patched
    .map((message, index) => (
      // Stripping an active run's stamp above does not make its known owner
      // legacy data. Never lend that reply to an unrelated failed run.
      message.role === "assistant" && !messages[index]!.runId && !isCompactionDivider(message)
        ? index
        : -1
    ))
    .filter(index => index >= 0)
    .reverse();

  // Only assign settled runs to persistent assistant messages.  Matching an
  // active run to an old assistant reply (positional misalignment after an
  // abort) would steal the runId and block the streaming bubble from ever
  // appearing.
  const settledAll = runs.filter(run =>
    matchesSettledRun(run.status) && !boundRunIds.has(run.id));

  // Exclude orphan runs (no exchange inside their window — the run failed before
  // the agent saved any assistant entry). Pairing them positionally would stamp
  // the failure onto the previous exchange and misalign every older pairing.
  // Guard: only trust window matching when at least one run actually matches a
  // exchange — legacy sessions without entry timestamps would orphan every run and
  // wipe out all pairing, so fall back to the positional behavior there.
  const timestamps = collectTurnTimestamps(messages);
  const settled = settledAll.some(run => runMatchesAnyTurn(run, timestamps))
    ? settledAll.filter(run => !isOrphanRun(run, timestamps))
    : settledAll;

  // The agent's save_callback persists each completed LLM call MID-RUN, so an
  // active (still-streaming) run can already have a partial assistant entry on
  // disk — it projects as the newest exchange. When exchanges outnumber settled runs,
  // the excess newest exchanges belong to the in-flight runs: leave them
  // unstamped. Stamping the newest settled run onto such an exchange instead
  // misaligns every older pairing (the real owner loses its runId/model badge)
  // and — worse — defeats streamingBubbleBase's duplicate detection, so the
  // frozen partial renders next to the growing live bubble (the "ABC, ABC →
  // ABC, ABCDE" duplicate).
  const canonicalRunIds = new Set(messages.filter(message => message.role === "assistant").map(message => message.runId));
  const inFlight = runs.filter(run => !matchesSettledRun(run.status) && !canonicalRunIds.has(run.id)).length;
  const skipNewest = Math.min(Math.max(turnIndices.length - settled.length, 0), inFlight);
  const assignable = turnIndices.slice(skipNewest);

  for (let i = 0; i < assignable.length && i < settled.length; i++) {
    const index = assignable[i]!;
    const run = settled[i]!;
    patched[index] = applyRunToMessage(patched[index]!, run);
  }
  return patched;
}

/** Turn a journal diagnostic into desktop-localized failure presentation. */
export function applyJournalRunOutcomes(messages: AgentMessage[]): AgentMessage[] {
  return messages.map(message =>
    message.status === "failed" && message.runError && !message.terminationNotice
      ? {
          ...message,
          terminationTitle: buildAgentFailureTitle(message.runError),
          terminationNotice: buildAgentFailureContent(message.runError),
        }
      : message,
  );
}

function applyRunToMessage(message: AgentMessage, run: StoredRun): AgentMessage {
  // An aborted exchange projects with no content and no reply time (the agent
  // saved no assistant entry). Stamp it with the run's end time — the actual
  // stop time — instead of the session-derived fallback. An exchange with real
  // content keeps its own recorded reply time.
  const isEmpty = !message.content.trim() && !message.segments?.length;
  const stopTime = isEmpty ? runEndedIso(run) : null;
  const stopped = run.status === "cancelled";
  const terminationNotice = stopped
    ? userStoppedNotice()
    : run.status === "failed"
      ? buildAgentFailureContent(run.errorMessage ?? "")
      : undefined;
  const terminationTitle = run.status === "failed"
    ? buildAgentFailureTitle(run.errorMessage ?? "")
    : undefined;
  return {
    ...message,
    runId: run.id,
    modelId: run.modelId ?? message.modelId,
    stopped,
    terminationNotice,
    terminationTitle,
    runError: run.errorMessage ?? message.runError,
    status: run.status === "failed" ? "failed" : (message.status ?? "complete"),
    durationMs: message.durationMs ?? runDurationMs(run),
    createdAt: stopTime ?? message.createdAt,
  };
}

/** The run's end (stop) time as ISO, or null when the run recorded none. */
function runEndedIso(run: StoredRun): string | null {
  const ms = run.endedAt ?? run.updatedAt;
  return typeof ms === "number" ? storedTimeToIso(ms) : null;
}

/**
 * Slack when matching an exchange's timestamp against a run's [start, end] window —
 * covers the gap between the user entry's save time and the run's creation,
 * plus minor clock granularity differences.
 */
const RUN_WINDOW_TOLERANCE_MS = 30_000;

/**
 * A run's [start, end] window (ms) widened by tolerance, or null when the run
 * carries no usable start time (legacy rows) and window matching can't apply.
 */
function runWindow(run: StoredRun): { start: number; end: number } | null {
  if (typeof run.startedAt !== "number")
    return null;
  const end = run.endedAt ?? run.updatedAt ?? run.startedAt;
  return { start: run.startedAt - RUN_WINDOW_TOLERANCE_MS, end: end + RUN_WINDOW_TOLERANCE_MS };
}

/**
 * Per-role message timestamps (Date.parse, finite only), collected ONCE per
 * load pass: the window matchers below run for every run, and re-parsing each
 * message's createdAt per run was O(runs × messages) of Date.parse work (L7).
 */
interface TurnTimestamps {
  assistant: number[];
  user: number[];
}

function collectTurnTimestamps(messages: AgentMessage[]): TurnTimestamps {
  const timestamps: TurnTimestamps = { assistant: [], user: [] };
  for (const message of messages) {
    if (message.role === "user") {
      const at = Date.parse(message.createdAt);
      if (Number.isFinite(at))
        timestamps.user.push(at);
      continue;
    }
    if (message.role === "assistant" && !isCompactionDivider(message)) {
      const at = Date.parse(message.createdAt);
      if (Number.isFinite(at))
        timestamps.assistant.push(at);
    }
  }
  return timestamps;
}

/** Whether any projected assistant reply's timestamp falls inside the run's window. */
function runMatchesAnyTurn(run: StoredRun, timestamps: TurnTimestamps): boolean {
  const window = runWindow(run);
  if (!window)
    return false;
  return timestamps.assistant.some(at => at >= window.start && at <= window.end);
}

/**
 * Whether the run's window covers a projected user exchange. A session whose FIRST
 * run failed has no assistant entry at all — the only message inside the run's
 * window is the user's — so window trust must not require an assistant reply.
 */
function runMatchesUserTurn(run: StoredRun, timestamps: TurnTimestamps): boolean {
  const window = runWindow(run);
  if (!window)
    return false;
  return timestamps.user.some(at => at >= window.start && at <= window.end);
}

/**
 * A settled run that produced NO assistant reply in the session JSONL — e.g. the
 * very first LLM call failed (insufficient credit, auth) or the run died before
 * any entry was saved. Positional newest-first pairing would stamp such a run
 * onto the PREVIOUS exchange, misaligning every older pairing; detect it by
 * timestamp so it can be excluded from pairing and surfaced as its own failure
 * bubble instead ({@link recoverFailedRuns}).
 */
function isOrphanRun(run: StoredRun, timestamps: TurnTimestamps): boolean {
  return runWindow(run) !== null && !runMatchesAnyTurn(run, timestamps);
}

/** Whether an exchange projected from session entries carries nothing renderable. */
function isEmptyTurn(message: AgentMessage): boolean {
  return message.role === "assistant"
    && !!message.runId
    && !message.content.trim()
    && !message.segments?.length;
}

/**
 * Fill empty aborted/failed exchanges from their run events (pure; events already
 * fetched). When a run is stopped mid-stream the agent's session JSONL holds no
 * assistant reply, so the exchange projects empty — but the partial text the model
 * streamed was persisted as run events. Recover it so a reload shows the
 * half-written answer instead of a blank "stopped" bubble. Exchanges that already
 * have content or segments are left untouched, so clean session-derived segments
 * are never overwritten by event-derived ones.
 */
export function applyRecoveredEvents(
  messages: AgentMessage[],
  eventsByRunId: Map<string, StoredRunEvent[]>,
): AgentMessage[] {
  return messages.map((message) => {
    if (!isEmptyTurn(message))
      return message;
    const events = eventsByRunId.get(message.runId!);
    if (!events?.length)
      return message;
    const projection = buildAssistantRunProjection(events);
    if (!projection.content.trim() && projection.segments.length === 0)
      return message;
    return {
      ...message,
      content: projection.content,
      segments: projection.segments.length > 0 ? projection.segments : message.segments,
      activityItems: projection.activityItems,
      outputTokens: projection.outputTokens,
    };
  });
}

/**
 * Recover partial content for aborted exchanges loaded via the agent session path.
 * Fetches events only for the empty exchanges, then applies {@link applyRecoveredEvents}.
 * Best-effort: any failure leaves the messages as-is.
 */
export async function recoverAbortedTurns(messages: AgentMessage[]): Promise<AgentMessage[]> {
  const emptyRunIds = messages.filter(isEmptyTurn).map(message => message.runId!);
  if (emptyRunIds.length === 0)
    return messages;
  try {
    const bulk = await listRunEventsBulk(emptyRunIds);
    return applyRecoveredEvents(messages, new Map(bulk));
  }
  catch {
    return messages;
  }
}

/**
 * Restore failure bubbles for runs that failed before the agent saved any
 * assistant entry (e.g. the first LLM call rejected with HTTP 402 insufficient
 * credit). The live send pipeline shows a failure bubble for these, but the
 * agent session JSONL — the reload source of truth — has no trace of the exchange,
 * so without this the error silently vanishes on a thread switch / reload. The
 * SQLite `runs` table still carries status + errorMessage, so rebuild the same
 * friendly failure bubble the live path showed and splice it in at the run's
 * chronological position (usually the tail; mid-history when the user retried
 * and a later exchange succeeded).
 */
export function recoverFailedRuns(messages: AgentMessage[], runs: StoredRun[]): AgentMessage[] {
  // Same guard as applyRunMetadata: when NO run window matches any exchange, the
  // session's timestamps are meaningless (legacy entries get load-time `now`
  // backfilled by the agent), every run would look like an orphan, and bubbles
  // would be spliced to the wrong end of history. Skip recovery there — it
  // degrades to main's behavior instead of inventing misplaced bubbles.
  // A user exchange counts as trust evidence too: a session whose FIRST run failed
  // (the "prompt acknowledgement omitted run_id" case) has no assistant entry
  // at all — the user's message is the only exchange inside the run's window.
  const timestamps = collectTurnTimestamps(messages);
  if (!runs.some(run => runMatchesAnyTurn(run, timestamps) || runMatchesUserTurn(run, timestamps)))
    return messages;
  const orphans = runs.filter(run =>
    run.status === "failed"
    && !messages.some(message => message.role === "assistant" && message.runId === run.id)
    && isOrphanRun(run, timestamps));
  if (orphans.length === 0)
    return messages;

  const out = [...messages];
  // `runs` arrive newest-first; insert oldest-first so earlier insertions
  // don't shift the position of later ones.
  for (const run of [...orphans].reverse()) {
    const bubble: AgentMessage = {
      // Stable id derived from the run — repeated reloads rebuild the same
      // bubble instead of keying off a random client id.
      id: `failed_${run.id}`,
      role: "assistant",
      authorKey: "author.researchCopilot",
      content: "",
      terminationNotice: buildAgentFailureContent(run.errorMessage ?? ""),
      terminationTitle: buildAgentFailureTitle(run.errorMessage ?? ""),
      status: "failed",
      runId: run.id,
      modelId: run.modelId ?? undefined,
      createdAt: runEndedIso(run) ?? new Date().toISOString(),
      durationMs: runDurationMs(run),
    };
    const bubbleAt = Date.parse(bubble.createdAt);
    const index = out.findIndex(message => Date.parse(message.createdAt) > bubbleAt);
    if (index === -1)
      out.push(bubble);
    else
      out.splice(index, 0, bubble);
  }
  return out;
}
