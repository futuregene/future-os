import type { Dispatch, SetStateAction } from "react";
import type { StoredRun, StoredRunEvent } from "../../integrations/storage/threadStore";
import type { AgentMessage, MessageSegment } from "./agentThreadTypes";
import { listRunEvents, listRunEventsBulk, listRuns, storedTimeToIso } from "../../integrations/storage/threadStore";
import { emitFutureEvent } from "../../lib/futureEvents";
import { buildAssistantRunProjection } from "./agentActivity";
import { matchesSettledRun } from "./agentMessageFormatters";

/** Apply a patch to the single message with `id`, leaving the rest untouched. */
export function patchMessage(
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>,
  id: string,
  patch: Partial<AgentMessage> | ((message: AgentMessage) => Partial<AgentMessage>),
) {
  setMessages(current =>
    current.map(message =>
      message.id === id
        ? { ...message, ...(typeof patch === "function" ? patch(message) : patch) }
        : message,
    ),
  );
}

/**
 * Fetch a run's events and project them for a live preview, honoring the
 * `shouldApply` guard (a stale async result is dropped by returning null). Emits
 * `file-tree-refresh` when tool activity appears — the agent may have created or
 * modified files. Shared prologue of the two live-preview writers below.
 */
async function projectRunForLivePreview(
  runId: string,
  shouldApply: () => boolean,
): Promise<ReturnType<typeof buildAssistantRunProjection> | null> {
  const events = await listRunEvents(runId);
  if (!shouldApply())
    return null;
  const projection = buildAssistantRunProjection(events);
  if (projection.activityItems.length > 0)
    emitFutureEvent("file-tree-refresh", undefined);
  return projection;
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
    const bubbleId = `stream_${runId}`;

    setMessages((current) => {
      // A persisted assistant message already carries this run — the run
      // settled and the thread was reloaded; don't resurrect a synthetic bubble.
      if (current.some(message => message.runId === runId && message.id !== bubbleId))
        return current;

      const content = projection.content.trim();
      // Also skip if the last assistant message already carries this
      // streaming content (save_callback may have persisted a mid-stream
      // assistant entry while the run is still active — the persisted
      // message has no runId, so the guard above doesn't catch it).
      const lastAssistant = [...current].reverse().find(m => m.role === "assistant");
      if (lastAssistant && !lastAssistant.runId && content && lastAssistant.content.includes(content.slice(0, 80)))
        return current;

      const existingIndex = current.findIndex(message => message.id === bubbleId);

      if (existingIndex === -1) {
        const bubble: AgentMessage = {
          id: bubbleId,
          role: "assistant",
          authorKey: "author.researchCopilot",
          content,
          status: "streaming",
          createdAt: new Date().toISOString(),
          activityItems: projection.activityItems,
          segments: projection.segments,
          thinkingActive: projection.thinkingActive,
          outputTokens: projection.outputTokens,
          // Feed MessageMeta's live elapsed timer so a re-attached run keeps
          // ticking instead of dropping its duration stat on switch-back.
          runStartedAt: runStartedAt ?? undefined,
          runId,
        };
        return [...current, bubble];
      }

      return current.map((message, index) =>
        index === existingIndex
          ? {
              ...message,
              activityItems: projection.activityItems,
              segments: projection.segments,
              content: content || message.content,
              thinkingActive: projection.thinkingActive,
              outputTokens: projection.outputTokens,
            }
          : message,
      );
    });
  }
  catch {
    // Live preview is best-effort; the final assistant message still lands when
    // the run settles and the thread reloads.
  }
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

    // Nothing renderable yet: no answer text, no tool activity, and no inline
    // segments. Reasoning-only turns DO carry a thinking segment, so this must
    // check segments too — otherwise the live thinking view (show-thinking on)
    // is swallowed until the first text/tool lands.
    if (!projection.content.trim() && projection.activityItems.length === 0 && projection.segments.length === 0)
      return;

    setMessages(current =>
      current.map(message =>
        message.id === pendingId
          ? {
              ...message,
              activityItems: projection.activityItems,
              // Live content is derived from the same events as segments, so the
              // two stay consistent — safe to render segments inline immediately.
              segments: projection.segments,
              content: projection.content.trim() ? projection.content : message.content,
              thinkingActive: projection.thinkingActive,
              // Tokens accumulate as each LLM call reports usage (lands at the
              // end of each call); shown as the real count, no estimate.
              outputTokens: projection.outputTokens,
            }
          : message,
      ),
    );
  }
  catch {
    // Streaming preview is best-effort. The final assistant message still
    // lands when the command returns.
  }
}

/**
 * Derive the renderable content + ordered segments from a run's events. Segments
 * are only trusted when the events actually carried the assistant text — the
 * stored reply (from the gRPC return) is otherwise authoritative, so legacy data
 * and text-only-via-gRPC turns fall back to flat content + activity list.
 */
export function deriveRenderFields(
  events: StoredRunEvent[],
  fallbackContent: string,
): { content: string; segments?: MessageSegment[]; outputTokens: number } {
  const projection = buildAssistantRunProjection(events);
  if (projection.content.trim()) {
    return {
      content: projection.content,
      segments: projection.segments,
      outputTokens: projection.outputTokens,
    };
  }
  return { content: fallbackContent, outputTokens: projection.outputTokens };
}

export async function safeListRunEvents(runId: string): Promise<StoredRunEvent[]> {
  try {
    return await listRunEvents(runId);
  }
  catch {
    return [];
  }
}

/**
 * Exact model run time from the persisted run; falls back to wall-clock since
 * the send anchor while the run is still settling. Null when neither is known.
 */
export function runDurationMs(run: StoredRun | null | undefined, fallbackStartMs?: number): number | null {
  if (run?.startedAt && run?.endedAt && run.endedAt >= run.startedAt) {
    return run.endedAt - run.startedAt;
  }
  if (typeof fallbackStartMs === "number") {
    return Math.max(0, Date.now() - fallbackStartMs);
  }
  return null;
}

/**
 * A compaction divider is projected as an assistant message but is not a real
 * turn — it carries no content and a single `compaction` segment. It must not
 * consume a run slot when aligning runs to turns.
 */
function isCompactionDivider(message: AgentMessage): boolean {
  return message.role === "assistant"
    && !message.content
    && message.segments?.length === 1
    && message.segments[0]?.kind === "compaction";
}

/**
 * Backfill run-derived status onto messages projected from agent session
 * entries. Agent JSONL only records message content, not a run's GUI-side
 * outcome (failed/cancelled/model) — that lives in the SQLite `runs` table. So a
 * reload from the agent path would otherwise show every turn as "complete",
 * losing the Retry/Continue affordance, the "stopped" marker, and the model
 * badge.
 *
 * `runs` arrive newest-first (`list_runs` orders by `created_at DESC`); real
 * assistant turns arrive oldest-first. Aligning from the newest end pairs the
 * most-recent turn with the most-recent run — the pairing that matters, since a
 * failure lands on the latest turn and `canRecover` only applies to the last
 * message. Counts can differ (a run that failed before emitting an assistant
 * entry, or a fork/import that synthesized a `completed` run); the extra items
 * on either end are simply ignored rather than force-matched into a misalignment.
 */
export function applyRunMetadata(messages: AgentMessage[], runs: StoredRun[]): AgentMessage[] {
  if (!runs.length)
    return messages;
  // Indices of real assistant turns, oldest-first; reversed to newest-first to
  // zip against the newest-first runs.
  const turnIndices = messages
    .map((message, index) => (message.role === "assistant" && !isCompactionDivider(message) ? index : -1))
    .filter(index => index >= 0)
    .reverse();

  // Only assign settled runs to persistent assistant messages.  An active
  // (still-streaming) run has no assistant entry on disk yet; its live
  // preview is inserted by upsertStreamingPreview.  Matching an active run
  // to an old assistant turn (positional misalignment after an abort) would
  // steal the runId and block the streaming bubble from ever appearing.
  const settled = runs.filter(run => matchesSettledRun(run.status));
  const patched = [...messages];
  for (let i = 0; i < turnIndices.length && i < settled.length; i++) {
    const index = turnIndices[i]!;
    const run = settled[i]!;
    const message = patched[index]!;
    // An aborted turn projects with no content and no reply time (the agent
    // saved no assistant entry). Stamp it with the run's end time — the actual
    // stop time — instead of the session-derived fallback. A turn with real
    // content keeps its own recorded reply time.
    const isEmpty = !message.content.trim() && !message.segments?.length;
    const stopTime = isEmpty ? runEndedIso(run) : null;
    patched[index] = {
      ...message,
      runId: run.id,
      modelId: run.modelId ?? message.modelId,
      stopped: run.status === "cancelled",
      status: run.status === "failed" ? "failed" : (message.status ?? "complete"),
      durationMs: message.durationMs ?? runDurationMs(run),
      createdAt: stopTime ?? message.createdAt,
    };
  }
  return patched;
}

/** The run's end (stop) time as ISO, or null when the run recorded none. */
function runEndedIso(run: StoredRun): string | null {
  const ms = run.endedAt ?? run.updatedAt;
  return typeof ms === "number" ? storedTimeToIso(ms) : null;
}

/** Whether a turn projected from session entries carries nothing renderable. */
function isEmptyTurn(message: AgentMessage): boolean {
  return message.role === "assistant"
    && !!message.runId
    && !message.content.trim()
    && !message.segments?.length;
}

/**
 * Fill empty aborted/failed turns from their run events (pure; events already
 * fetched). When a run is stopped mid-stream the agent's session JSONL holds no
 * assistant reply, so the turn projects empty — but the partial text the model
 * streamed was persisted as run events. Recover it so a reload shows the
 * half-written answer instead of a blank "stopped" bubble. Turns that already
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
 * Recover partial content for aborted turns loaded via the agent session path.
 * Fetches events only for the empty turns, then applies {@link applyRecoveredEvents}.
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

let clientIdCounter = 0;

export function clientId(prefix: string) {
  clientIdCounter += 1;
  return `${prefix}_${Date.now()}_${clientIdCounter}`;
}

export async function loadCurrentRun(threadId: string, runId: string) {
  try {
    const runs = await listRuns(threadId);
    return runs.find(run => run.id === runId) ?? null;
  }
  catch {
    return null;
  }
}
