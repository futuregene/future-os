import type { Dispatch, SetStateAction } from "react";
import type { StoredRun, StoredRunEvent } from "../../integrations/storage/threadStore";
import type { AgentMessage, MessageSegment } from "./agentThreadTypes";
import i18n from "../../i18n";
import { listRunEvents, listRunEventsBulk, listRuns } from "../../integrations/storage/threadStore";
import { buildAssistantRunProjection } from "./agentActivity";

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
    const events = await listRunEvents(runId);
    if (!shouldApply())
      return;

    const projection = buildAssistantRunProjection(events);
    const bubbleId = `stream_${runId}`;

    setMessages((current) => {
      // A persisted assistant message already carries this run — the run
      // settled and the thread was reloaded; don't resurrect a synthetic bubble.
      if (current.some(message => message.runId === runId && message.id !== bubbleId))
        return current;

      const content = projection.content.trim();
      const existingIndex = current.findIndex(message => message.id === bubbleId);

      if (existingIndex === -1) {
        const bubble: AgentMessage = {
          id: bubbleId,
          role: "assistant",
          author: i18n.t("agent:author.researchCopilot"),
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
    const events = await listRunEvents(runId);
    if (!shouldApply())
      return;

    const projection = buildAssistantRunProjection(events);

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

export async function restoreMessageActivities(messages: AgentMessage[], threadId: string) {
  const runs = await listRuns(threadId).catch(() => [] as StoredRun[]);
  const runById = new Map(runs.map(run => [run.id, run] as const));
  // Fetch events for all assistant runs in a single IPC call.
  const runIds = messages
    .filter(m => m.role === "assistant" && m.runId)
    .map(m => m.runId!);
  const eventsByRunId = new Map<string, StoredRunEvent[]>();
  if (runIds.length > 0) {
    try {
      const bulk = await listRunEventsBulk(runIds);
      for (const [rid, events] of bulk) {
        eventsByRunId.set(rid, events);
      }
    }
    catch {
      // Best-effort: keep empty projections on failure.
    }
  }

  const projectionByMessageId = new Map(
    messages.map((message) => {
      if (message.role !== "assistant" || !message.runId)
        return [message.id, null] as const;
      const events = eventsByRunId.get(message.runId);
      const projection = events ? buildAssistantRunProjection(events) : null;
      return [message.id, projection] as const;
    }),
  );

  return messages.map((message) => {
    const projection = projectionByMessageId.get(message.id);
    const run = message.runId ? runById.get(message.runId) ?? null : null;
    const meta: Partial<AgentMessage> = run
      ? { modelId: run.modelId ?? message.modelId, durationMs: runDurationMs(run), stopped: run.status === "cancelled" }
      : {};
    if (!projection)
      return { ...message, ...meta };
    // Trust event-derived inline ordering only when the events carried the
    // assistant text; otherwise keep the flat activity list (legacy fallback).
    const withSegments = projection.content.trim()
      ? { ...message, ...meta, activityItems: projection.activityItems, segments: projection.segments }
      : { ...message, ...meta, activityItems: projection.activityItems };
    return { ...withSegments, outputTokens: projection.outputTokens };
  });
}
