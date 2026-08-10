import type {
  ApprovalPayload,
  HistoryAttachment,
  HistoryEntry,
  HistoryMessage,
  StreamEvent,
  TimelineItem,
} from "./types";
import { messageText } from "./codec";

export interface TimelineState {
  items: TimelineItem[];
  seenEvents: Set<string>;
  currentRunId: string | null;
  streaming: boolean;
}

export function emptyTimeline(): TimelineState {
  return {
    items: [],
    seenEvents: new Set(),
    currentRunId: null,
    streaming: false,
  };
}

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

/** Accumulator for one user→assistant exchange (run) in history. */
interface HistoryExchange {
  user: TimelineItem | null;
  /** id of the run's (last) assistant entry — the reply bubble's stable id. */
  assistantId: string;
  /** Thinking and tool rows for the run, in entry order. */
  secondary: TimelineItem[];
  /** All of the run's streamed text, merged into one bubble. */
  finalText: string;
  runId?: string;
  durationMs?: number;
  outputTokens?: number;
}

function newHistoryExchange(user: TimelineItem | null): HistoryExchange {
  return { user, assistantId: "", secondary: [], finalText: "" };
}

/**
 * Display entries from `get_session_entries` — mirrors the desktop
 * entryProjection so history reads like the live transcript: each exchange
 * renders the user message, the run's thinking/tool rows, then the merged
 * reply bubble. The agent's JSONL groups a run as one user entry followed by
 * assistant entries (text + thinking + tool_calls) and tool result entries.
 */
export function timelineFromEntries(entries: HistoryEntry[]): TimelineState {
  const items: TimelineItem[] = [];
  let exchange: HistoryExchange | null = null;
  const flush = () => {
    if (!exchange) return;
    if (exchange.user) items.push(exchange.user);
    items.push(...exchange.secondary);
    const hasReply =
      exchange.finalText.trim().length > 0 ||
      exchange.durationMs != null ||
      exchange.outputTokens != null;
    if (hasReply) {
      items.push({
        id: exchange.assistantId || `history:assistant:${items.length}`,
        kind: "message",
        role: "assistant",
        text: exchange.finalText,
        ...(exchange.runId ? { runId: exchange.runId } : {}),
        ...(exchange.durationMs != null ? { durationMs: exchange.durationMs } : {}),
        ...(exchange.outputTokens != null ? { outputTokens: exchange.outputTokens } : {}),
      });
    }
    exchange = null;
  };
  entries.forEach((entry, index) => {
    if (entry.role === "user") {
      flush();
      const text = typeof entry.content === "string" ? entry.content : "";
      const attachments = (entry.meta?.attachments ?? []).filter(
        (attachment): attachment is HistoryAttachment =>
          !!attachment && typeof attachment.path === "string" && attachment.path.length > 0,
      );
      const user =
        text.trim().length > 0 || attachments.length > 0
          ? {
              id: `history:${entry.id ?? index}`,
              kind: "message" as const,
              role: "user" as const,
              text,
              ...(attachments.length > 0 ? { attachments } : {}),
            }
          : null;
      exchange = newHistoryExchange(user);
      return;
    }
    if (entry.role !== "assistant") return;
    if (!exchange) exchange = newHistoryExchange(null);
    const key = entry.id ?? `assistant:${index}`;
    exchange.assistantId = `history:${key}`;
    if (typeof entry.meta?.run_id === "string" && entry.meta.run_id)
      exchange.runId = entry.meta.run_id;
    if (typeof entry.output_tokens === "number") exchange.outputTokens = entry.output_tokens;
    if (typeof entry.duration_ms === "number") exchange.durationMs = entry.duration_ms;
    if (typeof entry.thinking === "string" && entry.thinking.trim().length > 0) {
      exchange.secondary.push({
        id: `history:${key}:thinking`,
        kind: "thinking",
        text: entry.thinking,
        complete: true,
        ...(exchange.runId ? { runId: exchange.runId } : {}),
      });
    }
    if (typeof entry.content === "string" && entry.content.trim().length > 0) {
      exchange.finalText = exchange.finalText
        ? `${exchange.finalText}\n\n${entry.content}`
        : entry.content;
    }
    if (entry.tool_calls?.length) {
      for (const [toolIndex, call] of entry.tool_calls.entries()) {
        const name = call.function?.name ?? "";
        if (!name) continue;
        exchange.secondary.push({
          id: `history:${key}:tool:${call.id ?? toolIndex}`,
          kind: "tool",
          toolId: call.id ?? `history-tool:${toolIndex}`,
          name,
          complete: true,
          ...(exchange.runId ? { runId: exchange.runId } : {}),
        });
      }
    }
  });
  flush();
  return { ...emptyTimeline(), items };
}

/**
 * Rebuild a session's timeline from a folded run projection (`projection.events`
 * returned by `get_events_since`). The projection is a coalesced replica of a
 * run whose event ring overflowed — individual events are in-order and carry
 * their own idx, so folding them through the normal reducer reproduces the
 * same transcript as if the run had streamed live. Each project contains the
 * whole run, so the caller replaces the session's cache wholesale.
 */
export function timelineFromProjection(events: StreamEvent[]): TimelineState {
  return events.reduce((state, event) => applyStreamEvent(state, event), emptyTimeline());
}

/**
 * Drop a run's timeline items so a replay of that run (from `get_events_since`)
 * can supersede them without duplicating the reply. History carries the run's
 * partial persisted entries (the agent appends them as it streams); the event
 * replay is authoritative. User bubbles and items of other runs are kept.
 */
export function stripRunItems(timeline: TimelineState, runId: string): TimelineState {
  return {
    ...timeline,
    items: timeline.items.filter(item =>
      item.kind === "message" && item.role === "user" ? true : item.runId !== runId,
    ),
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
 * Normalize `get_events_since` replay events (which the RPC serializes with
 * snake_case `run_id`) into the mobile `StreamEvent` shape (`runId`). The NATS
 * live mirror uses camelCase, so events arriving over the socket need no
 * normalization — only this backfill path does.
 */
export function normalizeReplayEvents(events: ReplayEventWire[] | undefined | null): StreamEvent[] {
  return (events ?? [])
    .filter((event): event is ReplayEventWire => !!event && typeof event === "object")
    .map(event => ({
      type: typeof event.type === "string" ? event.type : "",
      data: typeof event.data === "string" ? event.data : "",
      runId: typeof event.run_id === "string" ? event.run_id : "",
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object";
}

// Output (completion) tokens from a usage-bearing event — mirrors the desktop
// `usageOutputTokens` (desktop agentActivity): the streamed `usage` event nests the
// raw usage under a `usage` key, the `agent_end` total carries it the same way,
// and the field name is `completion_tokens` or `output_tokens` depending on the
// emitter. Returns 0 when absent so callers can guard with `> 0`.
function usageOutputTokens(data: Record<string, unknown>): number {
  const usage = isRecord(data.usage) ? data.usage : data;
  for (const key of ["completion_tokens", "output_tokens"]) {
    const value = usage[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return 0;
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
 * Append the run's streaming assistant placeholder when it doesn't exist yet.
 * The desktop keeps a streaming assistant bubble from send through run end;
 * mobile only creates one on agent_start/text_chunk — so a late-joining client
 * whose event-ring tail starts with thinking/usage events (agent_start already
 * rolled out of the agent's resume ring) would otherwise show no generating
 * indicator at all while the desktop does.
 */
function ensureAssistantItem(
  items: TimelineItem[],
  id: string,
  runId: string | undefined,
  startedAt: number,
): TimelineItem[] {
  if (items.some(item => item.id === id)) return items;
  return [
    ...items,
    { id, kind: "message", role: "assistant", text: "", runId, streaming: true, startedAt },
  ];
}

/**
 * Upsert a secondary item (thinking / tool) before the run's assistant
 * placeholder. The phone merges a run's streamed text into one bubble and
 * always renders it last — reasoning and tool work read above the answer,
 * the same overall shape the desktop renders inline — so the insertion
 * applies even once the placeholder holds text: a model that streams an
 * interim remark before its first tool call would otherwise push every
 * tool row below the reply. Without any placeholder (late join) the item
 * appends and ensureAssistantItem recreates the bubble after it.
 */
function upsertBeforeAssistant(
  items: TimelineItem[],
  placeholderId: string,
  id: string,
  create: () => TimelineItem,
  update: (item: TimelineItem) => TimelineItem,
): TimelineItem[] {
  const index = items.findIndex(item => item.id === id);
  if (index >= 0) return items.map((item, i) => (i === index ? update(item) : item));
  const placeholderIndex = items.findIndex(
    item => item.id === placeholderId && item.kind === "message",
  );
  const item = create();
  if (placeholderIndex < 0) return [...items, item];
  return [...items.slice(0, placeholderIndex), item, ...items.slice(placeholderIndex)];
}

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

  switch (event.type) {
    case "agent_start": {
      streaming = true;
      // The generating state lives on the assistant message itself (not a
      // separate timeline item), so it renders in the message footer — same
      // slot the copy button occupies once the run settles — and survives a
      // history resync, which carries no run indicator.
      const id = `assistant:${runId ?? event.idx ?? items.length}`;
      // Anchor the live timer to the run's real start (carried on the event),
      // not the local receipt time — a replayed agent_start would otherwise
      // restart the clock and understate the run's duration on settle.
      const eventStartedAt = runStartedAtMs(data);
      const startedAt = eventStartedAt ?? Date.now();
      items = upsertItem(
        items,
        id,
        () => ({
          id,
          kind: "message",
          role: "assistant",
          text: "",
          runId,
          streaming: true,
          startedAt,
        }),
        item =>
          item.kind === "message"
            ? { ...item, streaming: true, startedAt: eventStartedAt ?? item.startedAt ?? startedAt }
            : item,
      );
      break;
    }
    case "text_chunk": {
      const id = `assistant:${runId ?? event.idx ?? items.length}`;
      const chunk = textValue(data.text);
      items = upsertItem(
        items,
        id,
        () => ({
          id,
          kind: "message",
          role: "assistant",
          text: chunk,
          runId,
          streaming: true,
          startedAt: Date.now(),
        }),
        item =>
          item.kind === "message"
            ? {
                ...item,
                text: item.text + chunk,
                streaming: true,
                startedAt: item.startedAt ?? Date.now(),
              }
            : item,
      );
      break;
    }
    case "thinking_delta": {
      // A long reasoning prefix floods the agent's resume ring, so a client
      // joining mid-think may never see this run's agent_start. Host the
      // generating indicator on the assistant message anyway — the desktop
      // shows a streaming bubble for the whole run, reasoning included.
      const idKey = runId ?? event.idx ?? items.length;
      const assistantId = `assistant:${idKey}`;
      const id = `thinking:${idKey}`;
      const chunk = textValue(data.text);
      items = upsertBeforeAssistant(
        items,
        assistantId,
        id,
        () => ({ id, kind: "thinking", text: chunk, complete: false, runId }),
        item => (item.kind === "thinking" ? { ...item, text: item.text + chunk } : item),
      );
      items = ensureAssistantItem(items, assistantId, runId, Date.now());
      break;
    }
    case "thinking_end": {
      const id = `thinking:${runId ?? event.idx ?? items.length}`;
      items = items.map(item =>
        item.id === id && item.kind === "thinking" ? { ...item, complete: true } : item,
      );
      break;
    }
    case "tool_start": {
      const toolId = textValue(data.tool_id) || `tool-${event.idx ?? items.length}`;
      const id = `tool:${toolId}`;
      const assistantId = `assistant:${runId ?? event.idx ?? items.length}`;
      // Same ordering rule as thinking: tool rows precede the answer bubble,
      // even when the bubble already holds streamed text.
      items = upsertBeforeAssistant(
        items,
        assistantId,
        id,
        () => ({
          id,
          kind: "tool",
          toolId,
          name: textValue(data.tool_name) || "tool",
          complete: false,
          runId,
        }),
        item => item,
      );
      items = ensureAssistantItem(items, assistantId, runId, Date.now());
      break;
    }
    case "tool_end": {
      const toolId = textValue(data.tool_id);
      items = items.map(item =>
        item.kind === "tool" && item.toolId === toolId ? { ...item, complete: true } : item,
      );
      break;
    }
    case "usage": {
      // Per-call usage lands at the end of each LLM call. Accumulate onto the
      // run's assistant message (creating the placeholder when a late join put
      // a usage event first) so the live footer tracks the running total —
      // mirroring the desktop, which sums every call's usage unconditionally.
      // On settle the agent_end total replaces whatever accumulated here.
      const delta = usageOutputTokens(data);
      if (delta > 0) {
        const id = `assistant:${runId ?? event.idx ?? items.length}`;
        items = ensureAssistantItem(items, id, runId, Date.now());
        items = items.map(item =>
          item.id === id && item.kind === "message"
            ? { ...item, outputTokens: (item.outputTokens ?? 0) + delta }
            : item,
        );
      }
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
    case "error":
      items = [
        ...items,
        {
          id: `error:${runId ?? "none"}:${event.idx ?? items.length}`,
          kind: "notice",
          tone: "danger",
          text: textValue(data.error) || event.data,
          runId,
        },
      ];
      break;
    case "agent_end": {
      streaming = false;
      const endedAt = Date.now();
      const endTokens = usageOutputTokens(data);
      const endDurationMs = runDurationMs(data);
      // Settle the in-flight assistant message: clear the streaming flag and
      // stamp the run's wall-clock duration and output tokens. Both prefer the
      // authoritative totals carried on agent_end — a client that joined late
      // only saw the tail of the run's event ring, so its receipt-clock
      // duration and accumulated usage understate the run. (Desktop parity:
      // its settled footer reads these same run totals from the journal.)
      items = items.map(item => {
        if (item.kind !== "message" || item.role !== "assistant" || item.runId !== runId)
          return item;
        const next: TimelineItem = {
          ...item,
          streaming: false,
          durationMs: endDurationMs ?? (item.startedAt ? endedAt - item.startedAt : undefined),
        };
        if (next.kind === "message" && endTokens > 0) next.outputTokens = endTokens;
        return next;
      });
      break;
    }
    default:
      break;
  }

  return {
    items,
    seenEvents,
    currentRunId: event.runId ?? state.currentRunId,
    streaming,
  };
}

export function appendUserMessage(state: TimelineState, text: string): TimelineState {
  return {
    ...state,
    items: [
      ...state.items,
      {
        id: `local:${Date.now()}:${state.items.length}`,
        kind: "message",
        role: "user",
        text,
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
