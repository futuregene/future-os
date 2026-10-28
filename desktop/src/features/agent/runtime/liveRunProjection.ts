import type { AgentMessage, AssistantRunProjection, RunProjector } from "@future-os/thread-projection";
import type { Dispatch, SetStateAction } from "react";
import { compactionCheckpoints, createRunProjector } from "@future-os/thread-projection";
import { listRunEventsSince } from "../../../integrations/storage/threadStore";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { isCompactionDivider, isSupersededCompactionDivider } from "../messages/compactionMarkers";

// ── Incremental live-preview projection ──────────────────────────────────
// Runtime-update projection used to fetch the run's ENTIRE event log every
// tick and re-project it from scratch (O(n) per tick → O(n²) over a run, with
// every payload re-parsed and re-serialized across IPC). Instead, each run
// keeps a stateful projector here; every tick fetches only the events with
// `sequence > lastSequence` and ingests just those.
interface LiveProjectionEntry {
  projector: RunProjector;
  /** Null only between projector creation and the first ingest (same tick). */
  projection: AssistantRunProjection | null;
}

/** Max cached runs before evicting the least-recently-used projector. */
const LIVE_PROJECTION_CACHE_MAX = 8;

const liveProjectionCache = new Map<string, LiveProjectionEntry>();

const FILE_TREE_INVALIDATING_EVENTS = new Set([
  "toolcall_start",
  "tool_start",
  "tool_end",
  "tool_result",
  // An interrupted tool may have written files without emitting tool_end.
  "agent_end",
  "error",
]);

/**
 * Drop an incremental projector after the backend replaces its local event
 * log with an Agent projection snapshot. The next push rebuilds from the new
 * complete log instead of appending compressed snapshot text to a stale prefix.
 */
export function resetRunProjection(runId: string) {
  liveProjectionCache.delete(runId);
}

/**
 * Fetch a run's unseen events and advance its cached projector, honoring the
 * `shouldApply` guard (a stale async result is dropped by returning null). Emits
 * `file-tree-refresh` when tool activity appears — the agent may have created or
 * modified files. Shared prologue of the two live-preview writers below.
 */
async function projectRunForLivePreview(
  runId: string,
  shouldApply: () => boolean,
): Promise<AssistantRunProjection | null> {
  const cached = liveProjectionCache.get(runId) ?? null;
  const since = cached?.projector.lastSequence ?? -1;
  let events = await listRunEventsSince(runId, since);
  if (!shouldApply())
    return null;

  // A reset (or LRU eviction) during the IPC read invalidates its cursor.
  // Never ingest that tail into a new empty projector: its prefix would be
  // permanently missing even though subsequent sequence numbers look valid.
  if (cached && liveProjectionCache.get(runId) !== cached)
    return projectRunForLivePreview(runId, shouldApply);

  if (cached && events.length > 0 && events[0]!.sequence <= since) {
    // Sequence regressed under us — the agent realigned mid-stream (e.g. its
    // fallback restarted the event log for a new run). The incremental tail is
    // meaningless against the old projector: drop it and rebuild from the
    // full log once.
    liveProjectionCache.delete(runId);
    events = await listRunEventsSince(runId, -1);
    if (!shouldApply())
      return null;
  }

  let entry = liveProjectionCache.get(runId);
  if (!entry) {
    entry = { projector: createRunProjector(), projection: null };
  }
  // LRU touch; bound the cache so long sessions don't accumulate projectors.
  liveProjectionCache.delete(runId);
  liveProjectionCache.set(runId, entry);
  while (liveProjectionCache.size > LIVE_PROJECTION_CACHE_MAX) {
    const oldest = liveProjectionCache.keys().next().value;
    /* v8 ignore next 2 -- size > MAX (>= 1) guarantees a first key exists */
    if (oldest === undefined)
      break;
    liveProjectionCache.delete(oldest);
  }

  const previousSequence = entry.projector.lastSequence;
  const unseen = events.filter(event => event.sequence > previousSequence);
  if (unseen.length > 0 || !entry.projection)
    entry.projection = entry.projector.ingest(unseen);
  // Historical tool activity stays in every snapshot. Text/thinking/argument
  // deltas are not filesystem changes: invalidating on that history rescanned
  // every expanded directory every 2s for the remainder of a long reply.
  if (entry.projection.activityItems.length > 0
    && unseen.some(event => FILE_TREE_INVALIDATING_EVENTS.has(event.eventType))) {
    emitFutureEvent("file-tree-refresh", undefined);
  }
  return entry.projection;
}

/**
 * Reconcile the live streaming bubble against the current message list before
 * upserting. Returns the base array to upsert into, or null to add nothing.
 *
 * The agent's save_callback persists each completed LLM call mid-run, so a
 * reload during streaming surfaces a persisted assistant entry for THIS
 * in-flight exchange (no runId of its own — applyRunMetadata leaves in-flight
 * exchanges unstamped). Left in place, it renders alongside the live bubble as a
 * duplicate of the same reply. Detect it — the entry sits after the last user
 * message — and drop it, so the exchange renders once.
 */
export function streamingBubbleBase(
  current: AgentMessage[],
  runId: string,
  bubbleId: string,
  content: string,
  liveCheckpoints: ReadonlySet<string> = new Set(),
): AgentMessage[] | null {
  // A persisted assistant message already carries this run — the run settled
  // and the thread was reloaded; don't resurrect a synthetic bubble.
  if (current.some(message => message.role === "assistant" && message.runId === runId && message.id !== bubbleId))
    return null;

  const lastAssistantIdx = lastIndexOfRole(current, "assistant");
  const lastAssistant = lastAssistantIdx >= 0 ? current[lastAssistantIdx] : undefined;
  // Reaching here, no message carries `runId` (checked above) — so a runId on
  // the last assistant always belongs to ANOTHER run. If that assistant sits
  // in the in-flight exchange (after the last user message), its stamp can only be
  // a misaligned leftover (e.g. persisted by an older build) and the entry is
  // this exchange's mid-run snapshot regardless — don't let a stray runId shield
  // it from the dedup.
  if (lastAssistant && lastAssistant.runId !== runId) {
    // The mid-run persisted entry belongs to the in-flight exchange only when the
    // last user message precedes it; an assistant that appears before the last
    // user is an earlier completed exchange.
    const lastUserIdx = lastIndexOfRole(current, "user");
    const sameTurn = lastUserIdx >= 0 && lastUserIdx < lastAssistantIdx;

    // A mid-run snapshot for THIS in-flight exchange — the streaming bubble is
    // authoritative: it re-projects the exchange's thinking, tool activity AND
    // text from the run's event log, so dropping the snapshot loses nothing,
    // even when the snapshot carries no text yet (thinking/tools-only — its
    // `content` is empty but its segments render) or when no event has landed
    // in the log at all (the very first reattach tick may fire before the
    // collector persists the first chunks). A compaction divider is not a reply
    // snapshot, so it normally stays — except when the bubble renders the same
    // checkpoint, where keeping it would draw that divider twice.
    if (
      sameTurn
      && (!isCompactionDivider(lastAssistant)
        || isSupersededCompactionDivider(lastAssistant, liveCheckpoints))
    ) {
      return current.filter(message => message.id !== lastAssistant.id);
    }

    const persisted = lastAssistant.content.trim();
    // Prefix suppression is a legacy-only heuristic (5.4E): for canonical data
    // every settled exchange carries a runId, so the runId guard above already
    // handles a settled-run reload racing this tick. Applying the prefix test to
    // runId-bearing exchanges mis-kills repeated questions whose prior answer shares
    // a head with the live text ("continue" / "yes" / deterministic output), so
    // only fall back to it for legacy entries that carry no runId at all.
    if (
      !lastAssistant.runId
      && persisted
      && content
      && persisted.includes(content.slice(0, 80))
    ) {
      return null;
    }
  }
  return current;
}

/** Index of the last message with the given role (reverse scan — no throwaway role array). */
function lastIndexOfRole(messages: AgentMessage[], role: AgentMessage["role"]): number {
  for (let index = messages.length - 1; index >= 0; index--) {
    if (messages[index]!.role === role)
      return index;
  }
  return -1;
}

/**
 * Fold a pre-built live preview into restored thread history using the same
 * reconciliation as the incremental upsert path. This matters when switching
 * back to an active thread: the session JSONL may already contain a mid-run
 * assistant snapshot for the current exchange, which the preview must replace
 * rather than render beside.
 */
export function mergeStreamingPreview(
  current: AgentMessage[],
  preview: AgentMessage,
): AgentMessage[] {
  if (!preview.runId)
    return current;
  const base = streamingBubbleBase(
    current,
    preview.runId,
    preview.id,
    preview.content.trim(),
    compactionCheckpoints([preview]),
  );
  if (!base)
    return current;
  return [...base.filter(message => message.id !== preview.id), preview];
}

/** Runs whose live preview already reported a fallback (one warning per run). */
const previewFallbackWarned = new Set<string>();

/**
 * Record a live-preview fallback once per run.
 *
 * The fallback below keeps an active run's reply on screen as running, so a
 * failed projection is no longer silent — but the events themselves are still
 * missing, and that has to be visible to whoever debugs "the reply never grew"
 * instead of vanishing into a bare `catch`.
 */
export function warnLivePreviewFallback(runId: string, reason: string, error?: unknown) {
  if (previewFallbackWarned.has(runId))
    return;
  previewFallbackWarned.add(runId);
  console.warn(`[agent] live preview unavailable for run ${runId} (${reason}); showing history with a running reply`, error ?? "");
}

/**
 * Give the in-flight exchange its running state back when the live projection
 * produced no bubble for a run that is still executing.
 *
 * The agent saves an assistant entry after every model call, so a reload during
 * a run surfaces a mid-run snapshot as ordinary history: a `complete` reply
 * whose work folds into one row, no generating dot, no elapsed time, and — the
 * part that actually hurts — no stop button, because the whole view believes
 * the conversation is idle while the run keeps billing. This marks that
 * exchange, or opens the bubble the run would have started with when no entry
 * has landed yet, so the view tells the truth until `upsertStreamingPreview`
 * takes the same row over (it looks the reply up by runId + streaming).
 */
export function ensureLiveReply(
  messages: AgentMessage[],
  runId: string,
  runStartedAt: number | null,
): AgentMessage[] {
  // A live bubble already owns this run's reply: never re-mark or duplicate it.
  if (messages.some(message => message.role === "assistant"
    && message.status === "streaming"
    && message.runId === runId)) {
    return messages;
  }
  const lastAssistantIdx = lastIndexOfRole(messages, "assistant");
  const lastUserIdx = lastIndexOfRole(messages, "user");
  // Only the assistant AFTER the last user turn belongs to the in-flight
  // exchange; an earlier one is a settled reply this run did not write.
  if (lastAssistantIdx > lastUserIdx) {
    return messages.map((message, index): AgentMessage => (index === lastAssistantIdx
      // The snapshot's own content, segments and activity are kept — the reader
      // should not watch the work already on screen disappear — and only its
      // completion state is corrected.
      ? {
          ...message,
          status: "streaming",
          runId,
          runStartedAt: runStartedAt ?? message.runStartedAt,
          // A settle-time verdict cannot stand on a reply that is still being
          // written; the run's own outcome replaces it when the run ends.
          stopped: false,
          terminationNotice: undefined,
          terminationTitle: undefined,
          createdAt: runStartedAt != null ? new Date(runStartedAt).toISOString() : message.createdAt,
        }
      : message));
  }
  // Nothing in the transcript belongs to this run yet (entries are only saved
  // after a model call): show the bubble the run opened with.
  return [
    ...messages,
    {
      id: `stream_${runId}`,
      role: "assistant",
      authorKey: "author.researchCopilot",
      content: "",
      status: "streaming",
      createdAt: new Date(runStartedAt ?? Date.now()).toISOString(),
      activityItems: [],
      segments: [],
      thinkingActive: true,
      runStartedAt: runStartedAt ?? undefined,
      runId,
    },
  ];
}

/**
 * Render an in-flight run's live events as a streaming assistant bubble, keyed by
 * a stable `stream_<runId>` id. Unlike {@link updatePendingMessageFromRunEvents}
 * (which patches an existing optimistic bubble), this UPSERTS: it inserts the
 * bubble when missing and updates it in place otherwise, so it re-attaches to a
 * conversation the current view didn't start and survives store reloads that
 * replace the message array. Once a persisted assistant message for the run
 * exists (the run settled and was reloaded), it steps aside and adds nothing.
 */
export async function upsertStreamingPreview(
  runId: string,
  runStartedAt: number | null,
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>,
  shouldApply: () => boolean = () => true,
) {
  try {
    const projection = await projectRunForLivePreview(runId, shouldApply);
    if (!projection)
      return;
    const content = projection.content.trim();

    setMessages((current) => {
      // Switching back restores the local send's optimistic bubble from the
      // warm cache. Take over that still-streaming row instead of treating its
      // different UI id as evidence that this run has already settled.
      const bubbleId = current.find(message => message.role === "assistant"
        && message.runId === runId && message.status === "streaming")?.id ?? `stream_${runId}`;
      const base = streamingBubbleBase(
        current,
        runId,
        bubbleId,
        content,
        compactionCheckpoints([{ segments: projection.segments }]),
      );
      if (!base) {
        // Suppression means a persisted message already carries this run: the
        // settled-reload case when the run has ended, but a frozen mid-run
        // snapshot when it has not. This path only runs while the run is active,
        // so hand the row its running state back instead of rendering it as a
        // finished reply that cannot be stopped.
        warnLivePreviewFallback(runId, "persisted reply carries the run");
        return ensureLiveReply(current, runId, runStartedAt);
      }

      const existingIndex = base.findIndex(message => message.id === bubbleId);

      if (existingIndex === -1) {
        return [...base, streamingBubble(projection, runId, runStartedAt)];
      }

      const updated: AgentMessage = {
        ...base[existingIndex]!,
        // An older live bubble may have been stamped when the view reattached.
        // Keep its header aligned with the run's elapsed-time anchor.
        createdAt: runStartedAt != null
          ? new Date(runStartedAt).toISOString()
          : base[existingIndex]!.createdAt,
        activityItems: projection.activityItems,
        segments: projection.segments,
        content: content || base[existingIndex]!.content,
        thinkingActive: projection.thinkingActive,
        reconnecting: projection.reconnecting,
        outputTokens: projection.outputTokens,
      };
      // Replace in place — the old filter+append moved the bubble to the
      // array's end on every push (a DOM move when it wasn't last already).
      return base.map((message, index) => (index === existingIndex ? updated : message));
    });
  }
  catch (error) {
    // Live preview is best-effort: the final assistant message still lands when
    // the run settles and the thread reloads. Best-effort must not mean silent,
    // though — a run with no preview renders as a finished reply, so keep the
    // reply marked running and record why the projection was missing.
    warnLivePreviewFallback(runId, "projection threw", error);
    if (shouldApply())
      setMessages(current => ensureLiveReply(current, runId, runStartedAt));
  }
}

/**
 * Build a synthetic streaming bubble for an in-flight run WITHOUT touching
 * React state — the thread-load path folds it into the history array it
 * returns, so switching to an active conversation paints history + live
 * bubble in one render instead of history-then-bubble a frame apart (the
 * observer already holds both; the lag was purely two sequential
 * setMessages). Skips a projection with nothing renderable yet (no events
 * persisted) — the run is still in flight, so a later reattach tick adds
 * the bubble when content lands.
 */
export async function buildStreamingPreview(
  runId: string,
  runStartedAt: number | null = null,
): Promise<AgentMessage | null> {
  const projection = await projectRunForLivePreview(runId, () => true);
  /* v8 ignore next 2 -- shouldApply is always true here, so the projection
     only fails to materialize via a throw (which propagates) */
  if (!projection)
    return null;
  if (
    !projection.content.trim()
    && projection.activityItems.length === 0
    && projection.segments.length === 0
    && !projection.reconnecting
  ) {
    return null;
  }
  return streamingBubble(projection, runId, runStartedAt);
}

/** The synthetic live bubble for an in-flight run (shared by both writers). */
function streamingBubble(
  projection: AssistantRunProjection,
  runId: string,
  runStartedAt: number | null,
): AgentMessage {
  return {
    id: `stream_${runId}`,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: projection.content.trim(),
    status: "streaming",
    createdAt: new Date(runStartedAt ?? Date.now()).toISOString(),
    activityItems: projection.activityItems,
    segments: projection.segments,
    thinkingActive: projection.thinkingActive,
    reconnecting: projection.reconnecting,
    outputTokens: projection.outputTokens,
    // Feed MessageMeta's live elapsed timer so a re-attached run keeps
    // ticking instead of dropping its duration stat on switch-back.
    runStartedAt: runStartedAt ?? undefined,
    runId,
  };
}

export async function updatePendingMessageFromRunEvents(
  runId: string,
  pendingId: string,
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>,
  shouldApply: () => boolean = () => true,
) {
  try {
    const projection = await projectRunForLivePreview(runId, shouldApply);
    if (!projection)
      return;

    setMessages((current) => {
      const existingIndex = current.findIndex(m => m.id === pendingId);
      if (existingIndex === -1)
        return current;
      // Retry-only events are renderable even before the first token. An empty
      // resumed/terminal projection must also clear a previously shown retry.
      if (!projection.content.trim() && projection.activityItems.length === 0
        && projection.segments.length === 0 && !projection.reconnecting
        && !current[existingIndex]!.reconnecting) {
        return current;
      }
      const updated: AgentMessage = {
        ...current[existingIndex]!,
        activityItems: projection.activityItems,
        segments: projection.segments,
        content: projection.content.trim() ? projection.content : current[existingIndex]!.content,
        thinkingActive: projection.thinkingActive,
        reconnecting: projection.reconnecting,
        outputTokens: projection.outputTokens,
      };
      // Replace in place (see upsertStreamingPreview).
      return current.map((message, index) => (index === existingIndex ? updated : message));
    });
  }
  catch {
    // Streaming preview is best-effort. The final assistant message still
    // lands when the command returns.
  }
}
