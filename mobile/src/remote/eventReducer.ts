import type { ApprovalPayload, HistoryMessage, StreamEvent, TimelineItem } from "./types";
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
      const id = `run:${runId ?? event.idx ?? items.length}`;
      items = upsertItem(
        items,
        id,
        () => ({ id, kind: "run", startedAt: Date.now(), runId }),
        item => item,
      );
      break;
    }
    case "text_chunk": {
      const id = `assistant:${runId ?? event.idx ?? items.length}`;
      const chunk = textValue(data.text);
      items = upsertItem(
        items,
        id,
        () => ({ id, kind: "message", role: "assistant", text: chunk, runId }),
        item => (item.kind === "message" ? { ...item, text: item.text + chunk } : item),
      );
      break;
    }
    case "thinking_delta": {
      const id = `thinking:${runId ?? event.idx ?? items.length}`;
      const chunk = textValue(data.text);
      items = upsertItem(
        items,
        id,
        () => ({ id, kind: "thinking", text: chunk, complete: false, runId }),
        item => (item.kind === "thinking" ? { ...item, text: item.text + chunk } : item),
      );
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
      items = upsertItem(
        items,
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
      break;
    }
    case "tool_end": {
      const toolId = textValue(data.tool_id);
      items = items.map(item =>
        item.kind === "tool" && item.toolId === toolId ? { ...item, complete: true } : item,
      );
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
      const runItem = items.find(item => item.kind === "run" && item.runId === runId);
      const durationMs =
        runItem && runItem.kind === "run" ? Date.now() - runItem.startedAt : undefined;
      items = items
        .filter(item => item.kind !== "run" || item.runId !== runId)
        .map(item =>
          item.kind === "message" && item.role === "assistant" && item.runId === runId && durationMs
            ? { ...item, durationMs }
            : item,
        );
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
