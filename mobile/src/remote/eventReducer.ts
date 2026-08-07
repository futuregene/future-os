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

/** Display entries from `get_session_entries` — carries user attachments. */
export function timelineFromEntries(entries: HistoryEntry[]): TimelineState {
  const items: TimelineItem[] = [];
  entries.forEach((entry, index) => {
    if (entry.role !== "user" && entry.role !== "assistant") return;
    const text = typeof entry.content === "string" ? entry.content : "";
    const attachments = (entry.meta?.attachments ?? []).filter(
      (attachment): attachment is HistoryAttachment =>
        !!attachment && typeof attachment.path === "string" && attachment.path.length > 0,
    );
    if (!text.trim() && attachments.length === 0) return;
    items.push({
      id: `history:${entry.id ?? index}`,
      kind: "message",
      role: entry.role,
      text,
      ...(attachments.length > 0 ? { attachments } : {}),
      ...(typeof entry.duration_ms === "number" ? { durationMs: entry.duration_ms } : {}),
      ...(typeof entry.output_tokens === "number" ? { outputTokens: entry.output_tokens } : {}),
    });
  });
  return { ...emptyTimeline(), items };
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
 * Upsert a secondary item (thinking / tool) while keeping the run's assistant
 * placeholder last as long as it is still empty. The placeholder hosts the live
 * indicator and, eventually, the reply text — and chronologically the reasoning
 * and tool work precedes the answer, the order the desktop renders inline.
 * Without this the placeholder, created first by agent_start, would sit *above*
 * the thinking/tool cards and the answer text would appear over its own
 * reasoning. Once the placeholder holds text, new secondary items append (the
 * phone merges a run's text into one bubble, so true interleave isn't modeled).
 */
function upsertBeforeEmptyPlaceholder(
  items: TimelineItem[],
  placeholderId: string,
  id: string,
  create: () => TimelineItem,
  update: (item: TimelineItem) => TimelineItem,
): TimelineItem[] {
  const index = items.findIndex(item => item.id === id);
  if (index >= 0) return items.map((item, i) => (i === index ? update(item) : item));
  const placeholderIndex = items.findIndex(
    item => item.id === placeholderId && item.kind === "message" && item.text.trim().length === 0,
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
      items = upsertBeforeEmptyPlaceholder(
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
      // Same ordering rule as thinking: tool rows precede the answer bubble.
      items = upsertBeforeEmptyPlaceholder(
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
