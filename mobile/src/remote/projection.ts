import {
  compactionCheckpoints,
  createRunProjector,
  entriesToMessages,
  userMessageFromEvent,
  upsertUserMessage,
  type AgentActivityItem,
  type AgentMessage,
  type MessageSegment,
  type RunProjector,
} from "@future-os/thread-projection";
import type {
  ApprovalPayload,
  HistoryAttachment,
  HistoryEntry,
  HistoryMessage,
  StreamEvent,
  TimelineItem,
  TimelineSegment,
  TimelineToolRow,
} from "./types";
import { messageText } from "./codec";

export interface TimelineState {
  items: TimelineItem[];
  /** A tail read's paging checkpoint travels with its rows through replay.
   * The controller installs it only when the sync lane publishes those rows. */
  historyWindow?: { nextBefore: number; endOffset: number; hasMore: boolean };
  /** Persisted rows must not be re-appended as live messages after a page reset. */
  durableItemIds?: Set<string>;
  seenEvents: Set<string>;
  currentRunId: string | null;
  streaming: boolean;
  /** Session-wide admission fence, separate from an active model reply. */
  compacting?: boolean;
  /**
   * Per-run shared-projector accumulators for the live path. Kept out of the
   * render contract — the projector is stateful (slots/tool map), so folding a
   * run's events through it requires carrying the instance across events.
   */
  liveRuns?: Map<string, LiveRunState>;
}

/** Per-run live projection accumulator (internal, never rendered). */
interface LiveRunState {
  projector: RunProjector;
  assistantId: string;
  startedAt: number;
  streaming: boolean;
  durationMs?: number;
  failed: boolean;
  /** Raw agent error of a failed run — the bubble renders the friendly text. */
  error?: string;
}

export function emptyTimeline(): TimelineState {
  return {
    items: [],
    seenEvents: new Set(),
    currentRunId: null,
    streaming: false,
    liveRuns: new Map(),
  };
}

/**
 * Convert the shared projection's AgentMessage into the mobile render contract.
 * Assistant replies carry their ordered inline segments (thinking/tool/
 * compaction/text in stream order — desktop parity); user messages map to a
 * bubble with (optionally) attachment chips.
 */
export function messageToItems(message: AgentMessage): TimelineItem[] {
  const runId = message.runId ?? undefined;
  if (message.role === "user") {
    const text = message.content ?? "";
    const attachments = (message.attachments ?? [])
      .filter(attachment => !!attachment && attachment.path.length > 0)
      .map(toHistoryAttachment);
    if (!text.trim() && attachments.length === 0) return [];
    return [
      {
        id: message.id,
        kind: "message",
        role: "user",
        text,
        ...(attachments.length > 0 ? { attachments } : {}),
      },
    ];
  }
  if (message.role !== "assistant") return [];

  const content = message.content ?? "";
  const segments = (message.segments ?? []).map(segmentToTimeline);
  const hasVisible =
    content.trim().length > 0 ||
    segments.length > 0 ||
    message.status === "failed" ||
    message.stopped === true;
  if (!hasVisible && message.durationMs == null && message.outputTokens == null) return [];

  // The copyable/render text: the ordered text blocks, not the flattened
  // content field — desktop parity (copyableText joins the text segments).
  const text =
    segments.length > 0
      ? segments
          .filter(segment => segment.kind === "text")
          .map(segment => (segment.kind === "text" ? segment.text : ""))
          .join("\n\n")
      : content;

  const item: TimelineItem = {
    id: message.id,
    kind: "message",
    role: "assistant",
    text,
    ...(runId ? { runId } : {}),
    ...(segments.length > 0 ? { segments } : {}),
    ...(message.durationMs != null ? { durationMs: message.durationMs } : {}),
    ...(message.outputTokens != null && message.outputTokens > 0
      ? { outputTokens: message.outputTokens }
      : {}),
    ...(message.inputTokens != null && message.inputTokens > 0
      ? { inputTokens: message.inputTokens }
      : {}),
    ...(message.cacheReadTokens != null && message.cacheReadTokens > 0
      ? { cacheReadTokens: message.cacheReadTokens }
      : {}),
    ...(message.stopped ? { stopped: true } : {}),
    ...(message.truncated ? { truncated: true } : {}),
  };
  if (message.status === "failed") {
    item.failed = true;
    if (message.runError) item.error = message.runError;
  }
  return [item];
}

/** Display entries from `get_session_entries` — projection delegated to the
 * shared package (`entriesToMessages`), then mapped to the render contract. */
export function timelineFromEntries(entries: HistoryEntry[]): TimelineState {
  const messages = entriesToMessages(entries);
  const userRuns = new Map(
    entries.filter(entry => entry.role === "user").map(entry => [`m_${entry.id}`, entry.runId]),
  );
  const items = messages
    .flatMap(messageToItems)
    .map(item =>
      item.kind === "message" && item.role === "user" && userRuns.get(item.id)
        ? { ...item, runId: userRuns.get(item.id)! }
        : item,
    );
  return {
    ...emptyTimeline(),
    items,
    durableItemIds: new Set(items.map(item => item.id)),
  };
}

/** Render the model-context messages returned by get_messages. */
export function timelineFromHistory(messages: HistoryMessage[]): TimelineState {
  const items: TimelineItem[] = [];
  messages.forEach((message, index) => {
    const text = messageText(message.blocks);
    if (!text.trim() || (message.role !== "user" && message.role !== "assistant")) return;
    items.push({
      id: `history:${index}`,
      kind: "message",
      role: message.role,
      text,
      runId: message.runId ?? undefined,
    });
  });
  return { ...emptyTimeline(), items };
}

/** Merge durable attachment metadata into text-only live user events. */
export function mergeHistoryAttachments(
  live: TimelineState,
  durable: TimelineState,
): TimelineState {
  const attachmentsByText = new Map<string, (HistoryAttachment[] | undefined)[]>();
  for (const item of durable.items) {
    if (item.kind !== "message" || item.role !== "user") continue;
    const matches = attachmentsByText.get(item.text) ?? [];
    matches.push(item.attachments);
    attachmentsByText.set(item.text, matches);
  }
  const nextIndex = new Map<string, number>();
  const items = [...live.items];
  // The live cache can contain only the most recent part of the durable
  // transcript. Match from the end so repeated prompts attach to their latest
  // durable counterpart instead of an older bubble with the same text.
  for (let position = items.length - 1; position >= 0; position -= 1) {
    const item = items[position];
    if (!item || item.kind !== "message" || item.role !== "user") continue;
    const candidates = attachmentsByText.get(item.text);
    const index = nextIndex.get(item.text) ?? (candidates?.length ?? 0) - 1;
    nextIndex.set(item.text, index - 1);
    if (item.attachments?.length) continue;
    const attachments = candidates?.[index];
    if (attachments?.length) items[position] = { ...item, attachments };
  }
  return { ...live, items };
}

/**
 * Rebuild a session's timeline from a folded run projection (`projection.events`
 * returned by `get_events_since`). Folding the events through the normal
 * reducer reproduces the same transcript as if the run had streamed live.
 */
export function timelineFromProjection(events: StreamEvent[]): TimelineState {
  return applyStreamEvents(emptyTimeline(), events);
}

/**
 * Drop a run's timeline items (and its live projector accumulator) so a replay
 * of that run (from `get_events_since`) can supersede them without duplicating
 * the reply. User bubbles and items of other runs are kept.
 */
export function stripRunItems(timeline: TimelineState, runId: string): TimelineState {
  const liveRuns = timeline.liveRuns ? new Map(timeline.liveRuns) : undefined;
  if (liveRuns) liveRuns.delete(runId);
  return {
    ...timeline,
    items: timeline.items.filter(item =>
      item.kind === "message" && item.role === "user" ? true : item.runId !== runId,
    ),
    ...(liveRuns ? { liveRuns } : {}),
  };
}

/**
 * Upsert a run's live bubble, then drop the durable divider rows it supersedes.
 *
 * A checkpoint that opened the exchange is committed before that exchange has
 * any reply entry, so durable history renders it as a divider-only row; the
 * bubble renders the same checkpoint from the run's journal. Every path that
 * (re)builds the bubble must drop that row, so both the per-event and the
 * batched folding go through here.
 */
function upsertLiveAssistantItem(
  items: TimelineItem[],
  assistantId: string,
  item: TimelineItem,
): TimelineItem[] {
  const next = upsertItem(items, assistantId, () => item, () => item);
  // Hot path: this runs per event (and per batch flush). Only scan for the
  // checkpoint identity when a divider-only row is actually present.
  return next.some(isCompactionDividerRow)
    ? dropSupersededCompactionDividers(next, [item])
    : next;
}

/**
 * Drop durable rows the live render supersedes: a compaction must be drawn once.
 *
 * Until a run's exchange has its first reply entry, a checkpoint that opened
 * that exchange is all the durable history has — it projects as a message that
 * IS the divider. The live bubble carries the same checkpoint (see
 * `compactionCheckpoints`), so both would draw it. The durable row is the copy
 * to drop: the bubble re-renders it from the run's own journal.
 */
export function dropSupersededCompactionDividers(
  history: TimelineItem[],
  live: TimelineItem[],
): TimelineItem[] {
  // Only message rows render segments; the union's other members (notices,
  // approval cards) can never carry a divider.
  const liveCheckpoints = compactionCheckpoints(
    live.flatMap(item => (item.kind === "message" ? [item] : [])),
  );
  if (liveCheckpoints.size === 0) return history;
  // The live render never supersedes itself: the bubble is a divider-only row
  // too whenever the checkpoint is all the run has produced so far.
  const liveIds = new Set(live.map(item => item.id));
  return history.filter(item => !(
    item.kind === "message"
    && !liveIds.has(item.id)
    && isCompactionDividerRow(item)
    && (item.segments ?? []).some(segment =>
      segment.kind === "compaction"
      && !!segment.checkpointId
      && liveCheckpoints.has(segment.checkpointId),
    )
  ));
}

/** Durable history is the truth for settled standalone markers. When a merge
 * puts a checkpoint divider and a live placeholder for the same compaction
 * side by side — in either order — the placeholder must go: a running one
 * whose terminal was lost would otherwise sit below the completed divider
 * forever, and a settled one (folded late-start alias) is the same marker
 * under its operation id. */
export function foldLiveCompactionPlaceholdersIntoHistory(
  history: TimelineItem[],
  live: TimelineItem[],
): { history: TimelineItem[]; live: TimelineItem[] } {
  const checkpoints = new Set(
    history.flatMap(item =>
      item.kind === "message"
        ? (item.segments ?? []).flatMap((segment): string[] =>
          segment.kind === "compaction" && segment.checkpointId ? [segment.checkpointId] : [])
        : []),
  );
  if (checkpoints.size === 0) return { history, live };
  const dividerOnly = (item: TimelineItem) => item.kind === "message" && isCompactionDividerRow(item);
  const compactionSegments = (item: TimelineItem) =>
    item.kind === "message" ? (item.segments ?? []).filter(segment => segment.kind === "compaction") : [];
  const placeholder = (item: TimelineItem) =>
    dividerOnly(item)
    && item.id.startsWith("compaction:")
    && compactionSegments(item).every(segment =>
      !segment.checkpointId || checkpoints.has(segment.checkpointId));
  const startAliases = live.filter(item =>
    placeholder(item)
    && compactionSegments(item).every(segment => segment.status !== "running"));
  const aliasIds = new Set(startAliases.map(item => item.id));
  return {
    history: history.filter(item => !(dividerOnly(item) && aliasIds.has(item.id))),
    live: live.filter(item => !placeholder(item)),
  };
}

/** A row that renders nothing but a compaction divider (no reply text yet). */
function isCompactionDividerRow(item: TimelineItem): boolean {
  return item.kind === "message"
    && item.role === "assistant"
    && !item.text
    && item.segments?.length === 1
    && item.segments[0]?.kind === "compaction";
}

/** A raw replay event as the agent's `get_events_since` returns it (snake_case). */
export interface ReplayEventWire {
  type?: string;
  data?: string;
  run_id?: string;
  idx?: number;
  [key: string]: unknown;
}

/**
 * Normalize `get_events_since` replay events into the mobile `StreamEvent`
 * shape. The NATS live mirror and the desktop replay RPC both use camelCase
 * `runId`; legacy desktop bridges serialize snake_case `run_id` — accept both.
 */
export function normalizeReplayEvents(events: ReplayEventWire[] | undefined | null): StreamEvent[] {
  return (events ?? [])
    .filter((event): event is ReplayEventWire => !!event && typeof event === "object")
    .map(event => ({
      type: typeof event.type === "string" ? event.type : "",
      data: typeof event.data === "string" ? event.data : "",
      runId:
        typeof event.runId === "string"
          ? event.runId
          : typeof event.run_id === "string"
            ? event.run_id
            : "",
      idx: typeof event.idx === "number" ? event.idx : undefined,
    }));
}

function eventData(event: StreamEvent): Record<string, unknown> {
  try {
    return JSON.parse(event.data) as Record<string, unknown>;
  } catch {
    return {};
  }
}

function textValue(value: unknown): string {
  return typeof value === "string" ? value : "";
}

/** Epoch-ms run start carried on `agent_start` — the authoritative anchor, valid
 * even when the event is replayed to a late-joining client. */
function runStartedAtMs(data: Record<string, unknown>): number | undefined {
  const value = data.started_at_ms;
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : undefined;
}

/** Run wall-clock duration carried on `agent_end` (same value the desktop reads
 * back from the persisted journal). */
function runDurationMs(data: Record<string, unknown>): number | undefined {
  const value = data.duration_ms;
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : undefined;
}

function upsertItem(
  items: TimelineItem[],
  id: string,
  create: () => TimelineItem,
  update: (item: TimelineItem) => TimelineItem,
): TimelineItem[] {
  const index = items.findIndex(item => item.id === id);
  if (index < 0) return [...items, create()];
  return items.map((item, itemIndex) => (itemIndex === index ? update(item) : item));
}

/**
 * Apply one stream event to a timeline. Run events (agent_start/text_chunk/
 * thinking/tool/usage/agent_end) fold through the shared package's stateful
 * `createRunProjector`, so content accumulation, tool failure detection, tool
 * collapsing, compaction markers and truncated/stopped flags all come from the
 * single source of truth. UI-only state (streaming, the live timer anchor, the
 * settled duration) is layered on top of the projection snapshot.
 */
export function applyStreamEvent(state: TimelineState, event: StreamEvent): TimelineState {
  return applyEvent(state, event);
}

function applyEvent(state: TimelineState, event: StreamEvent, batch?: {
  seenEvents: Set<string>;
  deferRunSnapshot: boolean;
  data: Record<string, unknown>;
}): TimelineState {
  if (event.type === "ping") return state;
  const runId = event.runId ?? state.currentRunId ?? undefined;
  const key = event.runId != null && event.idx != null ? `${event.runId}:${event.idx}` : null;
  if (key && state.seenEvents.has(key)) return state;

  const seenEvents = batch?.seenEvents ?? new Set(state.seenEvents);
  if (key) seenEvents.add(key);
  const data = batch?.data ?? eventData(event);
  let items = state.items;
  let streaming = state.streaming;
  let compacting = state.compacting;
  let liveRuns = state.liveRuns;
  // A `_truncated` marker (text_chunk with no text) short-circuits the run
  // projection — see the text_chunk case below.
  let runEvents = true;

  switch (event.type) {
    case "compaction_started":
      compacting = true;
      break;
    case "compaction_committed":
    case "compaction_failed":
    case "compaction_unchanged":
      compacting = false;
      break;
    case "user_message": {
      const canonical = userMessageFromEvent(data);
      const text = canonical?.content ?? textValue(data.text);
      if (!text.trim() && !canonical?.attachments?.length) break;
      const user: Extract<TimelineItem, { kind: "message" }> = {
        id: canonical?.id ?? `user:${event.runId || `${Date.now()}:${items.length}`}`,
        kind: "message",
        role: "user",
        text,
        runId: canonical?.runId ?? (event.runId || undefined),
        ...(canonical?.attachments?.length
          ? { attachments: canonical.attachments.map(toHistoryAttachment) }
          : {}),
      };
      items = upsertUserMessage<TimelineItem>(items, user);
      break;
    }
    case "approval_request": {
      const payload = data as unknown as ApprovalPayload;
      if (payload.approval_request_id) {
        const id = `approval:${payload.approval_request_id}`;
        items = upsertItem(
          items,
          id,
          () => ({ id, kind: "approval", payload, runId }),
          item => item,
        );
      }
      break;
    }
    case "error": {
      // A `_truncated` relay-cap marker arrives as an error event too —
      // render the friendly sentinel, never the raw JSON blob.
      if (data._truncated === true) {
        items = [
          ...items,
          {
            id: `error:${runId ?? "none"}:${event.idx ?? items.length}`,
            kind: "notice",
            tone: "danger",
            text: "truncated",
            runId,
          },
        ];
        break;
      }
      // A run error settles the run as failed and pins the raw error onto its
      // assistant bubble, which renders the friendly failure text (desktop
      // parity: the failure text is the assistant content, not a banner).
      const raw = textValue(data.error) || event.data;
      if (runId) {
        const result = applyRunError(state, liveRuns, runId, raw);
        items = result.items;
        streaming = result.streaming;
        liveRuns = result.liveRuns;
      } else {
        items = [
          ...items,
          {
            id: `error:none:${event.idx ?? items.length}`,
            kind: "notice",
            tone: "danger",
            text: raw,
          },
        ];
      }
      break;
    }
    case "text_chunk": {
      // A relay-payload-cap truncation marker (`_truncated`, no text) must not
      // be folded into the projection — surface it as a friendly notice and
      // let the run keep streaming (later chunks merge into the bubble).
      if (data._truncated === true) {
        items = upsertTruncationNotice(items, runId);
        runEvents = false;
      }
      break;
    }
    default:
      break;
  }

  // Standalone compaction is session activity, not a reply. Materialize it
  // directly in both single-event and batched paths; it has no run accumulator
  // or run-scoped sequence (a later run can restart idx at zero).
  const standalone = event.type.startsWith("compaction_") && data.phase === "standalone";
  if (standalone) {
    items = applyStandaloneCompaction(items, event.type, data);
  } else if (runEvents && isRunEvent(event.type)) {
    const result = applyLiveEvent(state, runId, event, data, batch?.deferRunSnapshot);
    items = result.items;
    streaming = result.streaming;
    liveRuns = result.liveRuns;
  }

  return {
    ...state,
    items,
    seenEvents,
    currentRunId: standalone ? state.currentRunId : event.runId ?? state.currentRunId,
    streaming,
    compacting,
    liveRuns,
  };
}

/** A batch owns its dedup set and mutable projectors. Live frames materialize
 * one snapshot per contiguous run instead of per token; replay can yield/cancel
 * without mutating the still-visible committed timeline or its accumulators. */
export function createStreamEventBatch(initial: TimelineState) {
  const seenEvents = new Set(initial.seenEvents);
  const liveRuns = new Map<string, LiveRunState>();
  for (const [id, run] of initial.liveRuns ?? []) {
    liveRuns.set(id, { ...run, projector: run.projector.fork() });
  }
  let state: TimelineState = { ...initial, seenEvents, liveRuns };
  let pending: { runId: string | undefined } | null = null;
  const flush = () => {
    if (!pending) return;
    const run = state.liveRuns?.get(pending.runId ?? "__norun__");
    if (run) {
      const item = buildLiveAssistantItem(run, pending.runId, run.projector.snapshot(), run.durationMs);
      state = { ...state, items: upsertLiveAssistantItem(state.items, run.assistantId, item) };
    }
    pending = null;
  };
  return {
    append(event: StreamEvent) {
      if (event.type === "ping") return;
      const key = event.runId != null && event.idx != null ? `${event.runId}:${event.idx}` : null;
      if (key && seenEvents.has(key)) return;
      const runId = event.runId ?? state.currentRunId ?? undefined;
      const data = eventData(event);
      const deferRunSnapshot = isRunEvent(event.type)
        && !(event.type.startsWith("compaction_") && data.phase === "standalone")
        && !(event.type === "text_chunk" && data._truncated === true);
      // Materialize before an approval/user/error/notice or another run so
      // the same ordering and upsert semantics as single-event folding hold.
      if (pending && (!deferRunSnapshot || pending.runId !== runId)) flush();
      state = applyEvent(state, event, { seenEvents, deferRunSnapshot, data });
      if (deferRunSnapshot) pending = { runId };
    },
    finish() { flush(); return state; },
  };
}

/** Batch folding avoids a growing Set copy and render snapshot per event. */
export function applyStreamEvents(initial: TimelineState, events: StreamEvent[]): TimelineState {
  if (events.length === 0) return initial;
  const batch = createStreamEventBatch(initial);
  for (const event of events) batch.append(event);
  return batch.finish();
}

/** Cooperatively fold a large replay without committing partial cursors or UI.
 * Limits are checked between events; a single large payload/snapshot is not
 * preemptible. Count/byte bounds also guarantee yields under a fake clock. */
export async function applyReplayEvents(
  initial: TimelineState,
  events: StreamEvent[],
  options: { isCurrent?: () => boolean; onEvent?: (event: StreamEvent) => void } = {},
): Promise<TimelineState> {
  const isCurrent = options.isCurrent ?? (() => true);
  if (!isCurrent()) throw new Error("stale_sync_lane");
  if (events.length === 0) return initial;
  const batch = createStreamEventBatch(initial);
  let index = 0;
  while (index < events.length) {
    if (!isCurrent()) throw new Error("stale_sync_lane");
    const deadline = Date.now() + 8;
    let count = 0;
    let bytes = 0;
    do {
      const event = events[index++]!;
      options.onEvent?.(event);
      batch.append(event);
      count++;
      bytes += event.data.length * 2;
    } while (index < events.length && count < 512 && bytes < 256 * 1024 && Date.now() < deadline);
    if (index < events.length) await new Promise<void>(resolve => setTimeout(resolve, 0));
  }
  if (!isCurrent()) throw new Error("stale_sync_lane");
  return batch.finish();
}

function isRunEvent(type: string): boolean {
  return (
    type === "agent_start" ||
    type === "text_chunk" ||
    type === "thinking_start" ||
    type === "thinking_delta" ||
    type === "thinking_end" ||
    type === "tool_start" ||
    type === "tool_delta" ||
    type === "toolcall_delta" ||
    type === "tool_end" ||
    type === "tool_result" ||
    type === "usage" ||
    type === "compaction_end" ||
    type === "compaction_started" ||
    type === "compaction_committed" ||
    type === "compaction_failed" ||
    type === "compaction_unchanged" ||
    type === "agent_end"
  );
}

/** Operation-scoped placeholders become checkpoint-scoped history identities.
 * This preserves chronology across multiple compactions/runs and deduplicates
 * a terminal replay against a checkpoint already loaded from durable history. */
function applyStandaloneCompaction(
  items: TimelineItem[], type: string, data: Record<string, unknown>,
): TimelineItem[] {
  const operationId = textValue(data.operation_id);
  const checkpointId = textValue(data.checkpoint_id);
  if (!operationId && !checkpointId) return items;
  const pendingId = `compaction:${operationId}`;
  const id = type === "compaction_committed" && checkpointId ? `m_${checkpointId}` : pendingId;
  if (type === "compaction_unchanged") return items.filter(item => item.id !== pendingId);
  if (!["compaction_started", "compaction_committed", "compaction_failed"].includes(type)) return items;
  const status = type === "compaction_started" ? "running" : type === "compaction_failed" ? "failed" : "completed";
  const existing = items.find(item => item.id === id);
  // A replayed start must not undo a failed/settled marker for this operation.
  if (status === "running" && existing?.kind === "message"
    && existing.segments?.some(segment => segment.kind === "compaction" && segment.status !== "running")) return items;
  // A start delivered after its operation already settled (a replayed frame,
  // or a live frame that raced the history reload carrying the checkpoint)
  // must not append a second, permanently-running divider below the settled
  // one. Fold it into a trailing settled standalone marker: the durable
  // identity keeps its position, the pending id becomes an alias of it.
  // Standalone compactions are serialized by the agent (compaction_in_progress
  // fences the next one), so a settled divider that is still the LAST item
  // can only be this operation's — the durable copy never carries the
  // operation id, but no message could follow it before this start. Anything
  // after the divider (a prompt, a reply) means this start is a genuinely
  // new compaction and gets its own placeholder.
  if (status === "running" && !existing) {
    const last = items[items.length - 1];
    // A durable divider's segment carries no status (history rows omit it);
    // anything but an explicit "running" segment is settled.
    if (last?.kind === "message" && isCompactionDividerRow(last)
      && last.segments?.[0]?.kind === "compaction" && last.segments[0].status !== "running") {
      const segment = last.segments[0];
      return items.map((entry, index) => index === items.length - 1
        ? { ...last, id: pendingId, segments: [{ ...segment, status: segment.status ?? "completed" }] }
        : entry);
    }
  }
  const item: TimelineItem = {
    id, kind: "message", role: "assistant", text: "", streaming: false,
    segments: [{
      id: checkpointId ? `seg_${checkpointId}_compaction` : operationId,
      kind: "compaction", status,
      ...(checkpointId ? { checkpointId } : {}),
      ...(typeof data.tokens_before === "number" ? { tokensBefore: data.tokens_before } : {}),
      ...(typeof data.trigger === "string" ? { trigger: data.trigger } : {}),
      ...(typeof data.error === "string" ? { error: data.error } : {}),
    }],
  };
  if (existing) return items.filter(entry => entry.id !== pendingId || pendingId === id)
    .map(entry => entry.id === id ? item : entry);
  const pendingIndex = items.findIndex(entry => entry.id === pendingId);
  return pendingIndex < 0 ? [...items, item] : items.map((entry, index) => index === pendingIndex ? item : entry);
}

/** Fold one run event through the run's shared projector and rebuild the
 * assistant bubble from the projection snapshot. */
function applyLiveEvent(
  state: TimelineState,
  runId: string | undefined,
  event: StreamEvent,
  data: Record<string, unknown>,
  deferSnapshot = false,
): { items: TimelineItem[]; streaming: boolean; liveRuns: Map<string, LiveRunState> } {
  const liveRuns = state.liveRuns ?? new Map<string, LiveRunState>();
  const compactionEvent = event.type.startsWith("compaction_");
  const runKey = runId ?? "__norun__";
  let acc = liveRuns.get(runKey);
  if (!acc) {
    acc = {
      projector: createRunProjector({ preferEndTokens: true }),
      assistantId: `assistant:${runKey}`,
      startedAt: 0,
      streaming: false,
      failed: false,
    };
    liveRuns.set(runKey, acc);
  }
  if (event.type === "agent_start") {
    const eventStartedAt = runStartedAtMs(data);
    if (eventStartedAt) acc.startedAt = eventStartedAt;
    else if (!acc.startedAt) acc.startedAt = Date.now();
  }
  // Any run event other than agent_end means the run is still active — except a
  // compaction frame, which says nothing about whether a reply is in flight.
  if (event.type !== "agent_end" && !compactionEvent) acc.streaming = true;

  // Feed through the shared projector (agent_start is a no-op for it).
  let projection: ReturnType<RunProjector["snapshot"]> | undefined;
  if (deferSnapshot) acc.projector.append(toRunEvent(runKey, event));
  else projection = acc.projector.ingest([toRunEvent(runKey, event)]);

  let durationMs = acc.durationMs;
  if (event.type === "agent_end") {
    acc.streaming = false;
    const terminalState = textValue(data.state);
    acc.failed =
      terminalState === "error" ||
      terminalState === "failed" ||
      terminalState === "incomplete" ||
      data.reason === "incomplete" ||
      typeof data.error === "string";
    // Some bridges carry the raw error on the terminal event itself — keep it
    // so the bubble can render the friendly failure text.
    if (typeof data.error === "string" && data.error.trim() && !acc.error) {
      acc.error = data.error;
    }
    durationMs = runDurationMs(data) ?? (acc.startedAt ? Date.now() - acc.startedAt : undefined);
    acc.durationMs = durationMs;
  }
  let items = state.items;
  if (projection) {
    const assistantItem = buildLiveAssistantItem(acc, runId, projection, durationMs);
    items = upsertLiveAssistantItem(items, acc.assistantId, assistantItem);
  }
  return { items, streaming: compactionEvent ? state.streaming : acc.streaming, liveRuns };
}

function toRunEvent(
  runId: string,
  event: StreamEvent,
): {
  id: string;
  runId: string;
  eventType: string;
  payload: string | null;
  sequence: number;
  createdAt: number;
} {
  return {
    id: `${runId}:${event.idx ?? 0}`,
    runId,
    eventType: event.type,
    payload: event.data,
    sequence: event.idx ?? 0,
    createdAt: 0,
  };
}

function buildLiveAssistantItem(
  acc: LiveRunState,
  runId: string | undefined,
  projection: {
    content: string;
    segments: MessageSegment[];
    activityItems: AgentActivityItem[];
    outputTokens: number;
    stopped: boolean;
    truncated: boolean;
  },
  durationMs: number | undefined,
): TimelineItem {
  const segments = projection.segments.map(segmentToTimeline);
  // Copyable/render text: the ordered text blocks (desktop parity) — the
  // flattened content field is only a fallback for segment-less projections.
  const text =
    segments.length > 0
      ? segments
          .filter(segment => segment.kind === "text")
          .map(segment => (segment.kind === "text" ? segment.text : ""))
          .join("\n\n")
      : projection.content;
  const item: TimelineItem = {
    id: acc.assistantId,
    kind: "message",
    role: "assistant",
    text,
    ...(runId ? { runId } : {}),
    // Explicit streaming flag: true while live, false once the run settles
    // (agent_end) — the footer swaps the generating indicator for the copy
    // button exactly at that boundary.
    streaming: acc.streaming,
    ...(acc.streaming && acc.startedAt > 0 ? { startedAt: acc.startedAt } : {}),
    ...(segments.length > 0 ? { segments } : {}),
    ...(durationMs != null ? { durationMs } : {}),
    ...(projection.outputTokens > 0 ? { outputTokens: projection.outputTokens } : {}),
    ...(projection.stopped ? { stopped: true } : {}),
    ...(projection.truncated ? { truncated: true } : {}),
  };
  if (acc.failed) item.failed = true;
  if (acc.error) item.error = acc.error;
  return item;
}

/**
 * Settle a run as failed from its terminal `error` event and pin the raw error
 * onto the assistant bubble (which renders the friendly failure text, desktop
 * parity). Mirrors {@link applyLiveEvent}'s accumulator handling so an error
 * before any other run event still produces the bubble.
 */
function applyRunError(
  state: TimelineState,
  liveRuns: Map<string, LiveRunState> | undefined,
  runId: string,
  raw: string,
): { items: TimelineItem[]; streaming: boolean; liveRuns: Map<string, LiveRunState> } {
  const runs = liveRuns ?? new Map<string, LiveRunState>();
  const runKey = runId;
  let acc = runs.get(runKey);
  if (!acc) {
    acc = {
      projector: createRunProjector({ preferEndTokens: true }),
      assistantId: `assistant:${runKey}`,
      startedAt: 0,
      streaming: false,
      failed: false,
    };
    runs.set(runKey, acc);
  }
  acc.error = raw;
  acc.failed = true;
  acc.streaming = false;
  if (acc.durationMs == null && acc.startedAt > 0) acc.durationMs = Date.now() - acc.startedAt;
  const assistantItem = buildLiveAssistantItem(
    acc,
    runId,
    acc.projector.ingest([]),
    acc.durationMs,
  );
  const items = upsertItem(
    state.items,
    acc.assistantId,
    () => assistantItem,
    () => assistantItem,
  );
  return { items, streaming: acc.streaming, liveRuns: runs };
}

/** Map a shared MessageSegment onto the mobile bubble's inline segment union.
 * The shared segment's stable `id` is kept for React keys. */
function segmentToTimeline(segment: MessageSegment): TimelineSegment {
  switch (segment.kind) {
    case "text":
      return { id: segment.id, kind: "text", text: segment.text };
    case "thinking":
      return { id: segment.id, kind: "thinking", text: segment.text };
    case "activity": {
      const activity = segment.item;
      return {
        id: segment.id,
        kind: "tool",
        tool: {
          name: activity.kind,
          complete: activity.status !== "running",
          status: activity.status,
          ...(activity.detail ? { detail: activity.detail } : {}),
          ...(activity.count != null && activity.count > 1 ? { count: activity.count } : {}),
          ...(activity.children?.length
            ? { children: activity.children.map(activityToToolRow) }
            : {}),
        },
      };
    }
    case "compaction":
      return {
        id: segment.id,
        kind: "compaction",
        ...(segment.checkpointId ? { checkpointId: segment.checkpointId } : {}),
        ...(segment.tokensBefore ? { tokensBefore: segment.tokensBefore } : {}),
        ...(segment.trigger ? { trigger: segment.trigger } : {}),
        ...(segment.status ? { status: segment.status } : {}),
        ...(segment.error ? { error: segment.error } : {}),
      };
  }
}

function activityToToolRow(activity: AgentActivityItem): TimelineToolRow {
  return {
    name: activity.kind,
    complete: activity.status !== "running",
    status: activity.status,
    ...(activity.detail ? { detail: activity.detail } : {}),
  };
}

function toHistoryAttachment(attachment: {
  path: string;
  name: string;
  kind?: "image" | "file" | null;
}): HistoryAttachment {
  return { path: attachment.path, name: attachment.name, kind: attachment.kind ?? undefined };
}

export function appendUserMessage(
  state: TimelineState,
  text: string,
  attachments?: HistoryAttachment[],
): TimelineState {
  return {
    ...state,
    items: [
      ...state.items,
      {
        id: `local:${Date.now()}:${state.items.length}`,
        kind: "message",
        role: "user",
        text,
        ...(attachments?.length ? { attachments } : {}),
      },
    ],
  };
}

/**
 * Commit a prompt only after the desktop acknowledged it. The live
 * `user_message` mirror can race ahead of the acknowledgement, so enrich that
 * run-scoped bubble instead of appending a duplicate. Until this function is
 * called, the composer remains the sole owner of the unsent text/attachments.
 */
export function commitAcknowledgedUserMessage(
  state: TimelineState,
  input: {
    id: string;
    runId: string;
    text: string;
    attachments?: HistoryAttachment[];
  },
): TimelineState {
  const existingIndex = state.items.findIndex(
    item =>
      item.kind === "message" &&
      item.role === "user" &&
      (item.id === input.id ||
        (item.runId === input.runId && item.text.trim() === input.text.trim())),
  );
  if (existingIndex >= 0) {
    const existing = state.items[existingIndex];
    if (!existing || existing.kind !== "message") return state;
    const items = [...state.items];
    items[existingIndex] = {
      ...existing,
      runId: input.runId,
      ...(input.attachments?.length ? { attachments: input.attachments } : {}),
    };
    return { ...state, items };
  }
  return {
    ...state,
    items: [
      ...state.items,
      {
        id: input.id,
        kind: "message",
        role: "user",
        text: input.text,
        runId: input.runId,
        ...(input.attachments?.length ? { attachments: input.attachments } : {}),
      },
    ],
  };
}

export function markApprovalDecision(
  state: TimelineState,
  approvalId: string,
  decision: "approved" | "rejected" | "cancelled",
): TimelineState {
  return {
    ...state,
    items: state.items.map(item =>
      item.kind === "approval" && item.payload.approval_request_id === approvalId
        ? { ...item, decision }
        : item,
    ),
  };
}

/**
 * Surface the `_truncated` wire marker (a replay event whose data exceeded the
 * relay payload cap) as a muted notice in the timeline — the friendly
 * `chat.truncated` text the desktop sends, not the raw JSON blob. Idempotent:
 * one notice per run.
 */
export function upsertTruncationNotice(
  items: TimelineItem[],
  runId: string | undefined,
): TimelineItem[] {
  const id = `notice:truncated:${runId ?? "none"}`;
  const marker = (item: TimelineItem): item is Extract<TimelineItem, { kind: "notice" }> =>
    item.kind === "notice" && item.text === "truncated";
  if (items.some(marker)) return items;
  return [
    ...items,
    {
      id,
      kind: "notice",
      tone: "warning",
      text: "truncated",
      runId,
    },
  ];
}
