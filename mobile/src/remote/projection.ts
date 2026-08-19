import {
  createRunProjector,
  entriesToMessages,
  type AgentActivityItem,
  type AgentMessage,
  type MessageSegment,
  type RunProjector,
  type SessionEntry,
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
  seenEvents: Set<string>;
  currentRunId: string | null;
  streaming: boolean;
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
  const hasVisible = content.trim().length > 0 || segments.length > 0;
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
  if (message.status === "failed") item.failed = true;
  return [item];
}

/** History entries (mobile wire shape) → the shared session-entry shape. */
function toSessionEntries(entries: HistoryEntry[]): SessionEntry[] {
  return entries.map((entry, index) => {
    const role = entry.role === "assistant" || entry.role === "tool" ? entry.role : "user";
    const toolCalls = (entry.tool_calls ?? [])
      .filter(call => !!call && typeof call?.function?.name === "string")
      .map(call => ({
        id: call.id ?? `call_${index}`,
        function: {
          name: call.function!.name as string,
          arguments: call.function!.arguments,
        },
      }));
    return {
      id: entry.id ?? `entry_${index}`,
      role,
      content: typeof entry.content === "string" ? entry.content : "",
      ...(entry.thinking != null ? { thinking: entry.thinking } : {}),
      ...(toolCalls.length > 0 ? { tool_calls: toolCalls } : {}),
      ...(entry.meta
        ? {
            meta: {
              ...(typeof entry.meta.run_id === "string" ? { run_id: entry.meta.run_id } : {}),
              ...(entry.meta.attachments?.length
                ? {
                    attachments: entry.meta.attachments
                      .filter(a => !!a && typeof a.path === "string" && a.path.length > 0)
                      .map(a => ({ path: a.path, name: a.name, kind: a.kind ?? "file" })),
                  }
                : {}),
            },
          }
        : {}),
      ...(entry.timestamp ? { timestamp: entry.timestamp } : {}),
      ...(entry.output_tokens != null ? { output_tokens: entry.output_tokens } : {}),
      ...(entry.duration_ms != null ? { duration_ms: entry.duration_ms } : {}),
    };
  });
}

/** Display entries from `get_session_entries` — projection delegated to the
 * shared package (`entriesToMessages`), then mapped to the render contract. */
export function timelineFromEntries(entries: HistoryEntry[]): TimelineState {
  const messages = entriesToMessages(toSessionEntries(entries));
  // The desktop store's authoritative run outcome, mirrored onto entries by the
  // remote bridge (`run_status`, plus `run_error` for failed runs).
  const runOutcomes = new Map(
    entries
      .filter(
        entry => typeof entry.meta?.run_id === "string" && typeof entry.run_status === "string",
      )
      .map(
        entry =>
          [
            entry.meta!.run_id!,
            {
              status: entry.run_status!,
              error:
                typeof entry.run_error === "string" && entry.run_error.trim()
                  ? entry.run_error
                  : undefined,
              durationMs:
                typeof entry.run_duration_ms === "number" &&
                Number.isFinite(entry.run_duration_ms) &&
                entry.run_duration_ms >= 0
                  ? entry.run_duration_ms
                  : undefined,
            },
          ] as const,
      ),
  );
  const projected = messages.flatMap(message => {
    const projectedItems = messageToItems(message);
    const outcome = message.runId ? runOutcomes.get(message.runId) : undefined;
    if (!outcome) return projectedItems;
    return projectedItems.map(item =>
      item.kind === "message" && item.role === "assistant"
        ? {
            ...item,
            ...(outcome.status === "failed" ? { failed: true } : {}),
            ...(outcome.status === "cancelled" ? { stopped: true } : {}),
            ...(item.durationMs == null && outcome.durationMs != null
              ? { durationMs: outcome.durationMs }
              : {}),
          }
        : item,
    );
  });
  // A run whose first LLM call failed left NO assistant entry in the journal —
  // the desktop rebuilds its failure bubble from the runs table on reload
  // (recoverFailedRuns); mobile splices the same bubble right after the user
  // turn that triggered the run, so the failure survives a re-open.
  const assistantRunIds = new Set(
    projected
      .filter(
        (item): item is Extract<TimelineItem, { kind: "message" }> =>
          item.kind === "message" && item.role === "assistant" && typeof item.runId === "string",
      )
      .map(item => item.runId!),
  );
  // User entries open exchanges 1:1 in journal order (entriesToMessages) and
  // the shared projection ids them `m_<entry id>` — the same id this file's
  // toSessionEntries assigns — so a failed run's bubble anchors after its user
  // item without re-deriving the exchange grouping.
  const failuresByAnchor = new Map<
    string,
    { runId: string; error?: string; durationMs?: number }
  >();
  const unanchored: { runId: string; error?: string; durationMs?: number }[] = [];
  entries.forEach((entry, index) => {
    if (entry.role !== "user") return;
    const runId = typeof entry.meta?.run_id === "string" ? entry.meta.run_id : null;
    if (!runId) return;
    const outcome = runOutcomes.get(runId);
    if (outcome?.status !== "failed" || assistantRunIds.has(runId)) return;
    const failure = { runId, error: outcome.error, durationMs: outcome.durationMs };
    const text = typeof entry.content === "string" ? entry.content : "";
    if (text.trim() || (entry.meta?.attachments?.length ?? 0) > 0) {
      failuresByAnchor.set(`m_${entry.id ?? `entry_${index}`}`, failure);
    } else {
      unanchored.push(failure);
    }
  });
  const toFailureBubble = (failure: {
    runId: string;
    error?: string;
    durationMs?: number;
  }): TimelineItem => ({
    id: `failed_${failure.runId}`,
    kind: "message",
    role: "assistant",
    text: "",
    runId: failure.runId,
    failed: true,
    ...(failure.error ? { error: failure.error } : {}),
    ...(failure.durationMs != null ? { durationMs: failure.durationMs } : {}),
  });
  const items: TimelineItem[] = [];
  for (const item of projected) {
    items.push(item);
    const failure = failuresByAnchor.get(item.id);
    if (failure) items.push(toFailureBubble(failure));
  }
  for (const failure of unanchored) items.push(toFailureBubble(failure));
  return { ...emptyTimeline(), items };
}

/** Message-shaped history fallback (old desktops without get_session_entries). */
export function timelineFromHistory(messages: HistoryMessage[]): TimelineState {
  const items: TimelineItem[] = [];
  messages.forEach((message, index) => {
    const text = messageText(message.content);
    if (!text.trim() || (message.role !== "user" && message.role !== "assistant")) return;
    items.push({
      id: `history:${index}`,
      kind: "message",
      role: message.role,
      text,
      runId: message.run_id,
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
  return events.reduce((state, event) => applyStreamEvent(state, event), emptyTimeline());
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
  if (event.type === "ping") return state;
  const runId = event.runId ?? state.currentRunId ?? undefined;
  const key = event.runId != null && event.idx != null ? `${event.runId}:${event.idx}` : null;
  if (key && state.seenEvents.has(key)) return state;

  const seenEvents = new Set(state.seenEvents);
  if (key) seenEvents.add(key);
  const data = eventData(event);
  let items = state.items;
  let streaming = state.streaming;
  let liveRuns = state.liveRuns;
  // A `_truncated` marker (text_chunk with no text) short-circuits the run
  // projection — see the text_chunk case below.
  let runEvents = true;

  switch (event.type) {
    case "user_message": {
      // The desktop observer mirrors prompts sent from ANY client (desktop,
      // TUI, another phone), so every device renders the user bubble live.
      // Dedup mirrors the desktop rule (useThreadMessages): skip when the
      // last user bubble has identical text — that is this device's own
      // optimistic send re-delivered through the mirror.
      const text = textValue(data.text);
      if (!text.trim()) break;
      let lastUser: TimelineItem | undefined;
      for (let i = items.length - 1; i >= 0; i -= 1) {
        const item = items[i];
        if (!item) continue;
        if (item.kind === "message" && item.role === "user") {
          lastUser = item;
          break;
        }
      }
      if (lastUser && lastUser.kind === "message" && lastUser.text.trim() === text.trim()) break;
      items = [
        ...items,
        {
          id: `user:${Date.now()}:${items.length}`,
          kind: "message",
          role: "user",
          text,
          runId,
        },
      ];
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

  // Run events flow through the shared projector.
  if (runEvents && isRunEvent(event.type)) {
    const result = applyLiveEvent(state, runId, event, data);
    items = result.items;
    streaming = result.streaming;
    liveRuns = result.liveRuns;
  }

  return {
    items,
    seenEvents,
    currentRunId: event.runId ?? state.currentRunId,
    streaming,
    liveRuns,
  };
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
    type === "agent_end"
  );
}

/** Fold one run event through the run's shared projector and rebuild the
 * assistant bubble from the projection snapshot. */
function applyLiveEvent(
  state: TimelineState,
  runId: string | undefined,
  event: StreamEvent,
  data: Record<string, unknown>,
): { items: TimelineItem[]; streaming: boolean; liveRuns: Map<string, LiveRunState> } {
  const liveRuns = state.liveRuns ?? new Map<string, LiveRunState>();
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
  // Any run event other than agent_end means the run is still active.
  if (event.type !== "agent_end") acc.streaming = true;

  // Feed through the shared projector (agent_start is a no-op for it).
  const projection = acc.projector.ingest([toRunEvent(runKey, event)]);

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
  const assistantItem = buildLiveAssistantItem(acc, runId, projection, durationMs);
  const items = upsertItem(
    state.items,
    acc.assistantId,
    () => assistantItem,
    () => assistantItem,
  );
  return { items, streaming: acc.streaming, liveRuns };
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
    ...(acc.streaming ? { startedAt: acc.startedAt } : {}),
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
        ...(segment.tokensBefore ? { tokensBefore: segment.tokensBefore } : {}),
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
