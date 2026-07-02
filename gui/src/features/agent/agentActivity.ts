import type { StoredRunEvent } from "../../integrations/storage/threadStore";
import type { AgentActivityItem, AgentActivityKind, MessageSegment } from "./agentThreadTypes";
import { isRecord, singleLine } from "../../lib/objects";

interface AssistantRunProjection {
  activityItems: AgentActivityItem[];
  content: string;
  /** Text and activity in chronological order — drives inline rendering. */
  segments: MessageSegment[];
  /**
   * Tokens this reply actually generated: summed completion tokens across
   * every LLM call in the run (0 when the provider returned no usage).
   */
  outputTokens: number;
  /**
   * The model's reasoning/thinking text for this turn (empty when none). Blocks
   * separated by blank lines; rendered only when the "show thinking" setting is
   * on. Extracted from `thinking_delta` events — always captured, gated at render.
   */
  thinking: string;
}

/**
 * Ordered placeholder: a text run or a reference to a tool by id. The tool's
 * latest state lives in the `toolActivities` map; the slot only fixes position.
 */
type Slot
  = | { type: "text"; text: string }
    | { type: "thinking"; text: string }
    | { type: "tool"; id: string };

interface ToolActivity {
  id: string;
  kind: Exclude<AgentActivityKind, "thinking">;
  status: AgentActivityItem["status"];
  target?: string;
  detail?: string;
  argsText?: string;
  order: number;
}

const SUPPORTED_TOOL_NAMES = new Set(["read", "bash", "edit", "write"]);

export function buildAssistantRunProjection(events: StoredRunEvent[]): AssistantRunProjection {
  const sortedEvents = [...events].sort((a, b) => a.sequence - b.sequence);
  const toolActivities = new Map<string, ToolActivity>();
  // Ordered timeline of the turn. Text accumulates into the open text slot;
  // each tool call pins a slot at the point it started.
  const slots: Slot[] = [];
  const slottedToolIds = new Set<string>();
  let openText: Extract<Slot, { type: "text" }> | null = null;
  // The currently-open reasoning block; thinking_delta text appends here so each
  // block keeps its own position in the timeline (interleaved with text/tools).
  let openThinking: Extract<Slot, { type: "thinking" }> | null = null;
  let content = "";
  let thinking = false;
  let sawVisibleWork = false;
  let activeToolCallId: string | null = null;
  // Output tokens: prefer summing per-call `usage` events; fall back to the
  // `agent_end` total only when no per-call usage was streamed.
  let usageOutputSum = 0;
  let sawUsageEvent = false;
  let agentEndOutput = 0;

  for (const event of sortedEvents) {
    const payload = parseEventPayload(event.payload);

    if (event.eventType === "usage") {
      usageOutputSum += usageOutputTokens(payload);
      sawUsageEvent = true;
      continue;
    }

    if (event.eventType === "agent_end") {
      agentEndOutput = usageOutputTokens(payload);
      continue;
    }

    if (event.eventType === "text_chunk") {
      const text = textFromPayload(payload);
      content += text;
      // Visible text ends any open reasoning block; later thinking opens a new one.
      openThinking = null;
      if (!openText) {
        openText = { type: "text", text: "" };
        slots.push(openText);
      }
      openText.text += text;
      if (text.trim()) {
        sawVisibleWork = true;
      }
      continue;
    }

    if (event.eventType === "thinking_start") {
      thinking = true;
      // Open a new reasoning slot at this point in the timeline. A text run ends
      // here so the block sits between the surrounding text/tools, not hoisted.
      openThinking = { type: "thinking", text: "" };
      slots.push(openThinking);
      openText = null;
      continue;
    }

    if (event.eventType === "thinking_delta") {
      const text = textFromPayload(payload);
      if (text) {
        // Tolerate a delta without a preceding start by opening a block lazily.
        if (!openThinking) {
          openThinking = { type: "thinking", text: "" };
          slots.push(openThinking);
          openText = null;
        }
        openThinking.text += text;
      }
      continue;
    }

    if (event.eventType === "thinking_end") {
      thinking = false;
      openThinking = null;
      continue;
    }

    if (event.eventType === "toolcall_start" || event.eventType === "tool_start") {
      const tool = toolFromPayload(payload, event.sequence);
      if (tool) {
        activeToolCallId = tool.id;
        toolActivities.set(tool.id, {
          ...toolActivities.get(tool.id),
          ...tool,
          status: "running",
        });
        if (!slottedToolIds.has(tool.id)) {
          slots.push({ type: "tool", id: tool.id });
          slottedToolIds.add(tool.id);
        }
        // A tool call ends the current text/thinking run; later output opens a
        // fresh slot so ordering is preserved.
        openText = null;
        openThinking = null;
        sawVisibleWork = true;
      }
      continue;
    }

    if (event.eventType === "toolcall_delta") {
      if (!activeToolCallId)
        continue;

      const existing = toolActivities.get(activeToolCallId);
      if (!existing)
        continue;

      const nextArgsText = `${existing.argsText ?? ""}${textFromPayload(payload)}`;
      const target = targetFromToolArgs(existing.kind, nextArgsText);
      toolActivities.set(activeToolCallId, {
        ...existing,
        argsText: nextArgsText,
        ...(target
          ? {
              detail: target,
              target: singleLine(target),
            }
          : {}),
      });
      continue;
    }

    if (event.eventType === "tool_end" || event.eventType === "tool_result") {
      const tool = toolFromPayload(payload, event.sequence);
      if (!tool)
        continue;

      const toolId = explicitToolId(payload) ?? latestRunningToolId(toolActivities, tool.kind) ?? tool.id;
      const existing = toolActivities.get(toolId);
      if (activeToolCallId === toolId)
        activeToolCallId = null;
      toolActivities.set(toolId, {
        ...existing,
        ...tool,
        id: toolId,
        status: hasToolError(payload) ? "failed" : "completed",
        order: existing?.order ?? tool.order,
      });
      // A result without a preceding start still deserves a slot.
      if (!slottedToolIds.has(toolId)) {
        slots.push({ type: "tool", id: toolId });
        slottedToolIds.add(toolId);
      }
      sawVisibleWork = true;
    }
  }

  const segments = buildSegments(slots, toolActivities);

  // Flat activity list kept for back-compat (legacy render path / callers).
  const items = collapseToolActivities([...toolActivities.values()].sort((a, b) => a.order - b.order));
  if (thinking && !content.trim() && !sawVisibleWork) {
    const thinkingItem: AgentActivityItem = { id: "thinking", kind: "thinking", status: "running" };
    items.unshift(thinkingItem);
    segments.unshift({ kind: "activity", id: thinkingItem.id, item: thinkingItem });
  }

  // Concatenated reasoning (blocks joined by blank lines) — the inline segments
  // carry the ordered form; this is the whole-turn text for any non-inline use.
  const thinkingText = slots
    .filter((slot): slot is Extract<Slot, { type: "thinking" }> => slot.type === "thinking")
    .map(slot => slot.text.trim())
    .filter(Boolean)
    .join("\n\n");

  return {
    activityItems: items,
    content,
    segments,
    outputTokens: sawUsageEvent ? usageOutputSum : agentEndOutput,
    thinking: thinkingText,
  };
}

function numberFromPayload(payload: unknown, keys: string[]): number {
  if (!isRecord(payload))
    return 0;
  for (const key of keys) {
    const value = payload[key];
    if (typeof value === "number" && Number.isFinite(value))
      return value;
  }
  return 0;
}

/**
 * Output (completion) tokens from a usage-bearing event. The gRPC StreamEvent
 * nests the raw `Usage` under a `usage` key (`{type,usage:{completion_tokens}}`),
 * matching how the TUI reads it; we also tolerate a flat shape just in case.
 */
function usageOutputTokens(payload: unknown): number {
  if (!isRecord(payload))
    return 0;
  const usage = isRecord(payload.usage) ? payload.usage : payload;
  return numberFromPayload(usage, ["completion_tokens", "output_tokens"]);
}

/**
 * Walk the ordered slots into renderable segments. Adjacent tool slots (ignoring
 * whitespace-only text between them) are grouped with the same collapse rules as
 * the flat list, so a burst of edits still reads as "已编辑 N 个文件" — but real
 * prose between tools keeps them as separate inline lines.
 */
function buildSegments(
  slots: Slot[],
  toolActivities: Map<string, ToolActivity>,
): MessageSegment[] {
  const segments: MessageSegment[] = [];
  let index = 0;

  while (index < slots.length) {
    const slot = slots[index];
    if (!slot)
      break;

    if (slot.type === "text") {
      if (slot.text.trim()) {
        segments.push({ kind: "text", id: `text_${index}`, text: slot.text });
      }
      index += 1;
      continue;
    }

    if (slot.type === "thinking") {
      if (slot.text.trim()) {
        segments.push({ kind: "thinking", id: `thinking_${index}`, text: slot.text });
      }
      index += 1;
      continue;
    }

    // Gather a run of adjacent tool slots, hopping over whitespace-only text.
    const run: ToolActivity[] = [];
    let cursor = index;
    while (cursor < slots.length) {
      const current = slots[cursor];
      if (!current)
        break;
      if (current.type === "tool") {
        const tool = toolActivities.get(current.id);
        if (tool)
          run.push(tool);
        cursor += 1;
        continue;
      }
      if (!current.text.trim()) {
        cursor += 1;
        continue;
      }
      break;
    }

    for (const item of collapseToolActivities(run)) {
      segments.push({ kind: "activity", id: item.id, item });
    }
    index = cursor;
  }

  return segments;
}

function latestRunningToolId(
  toolActivities: Map<string, ToolActivity>,
  kind: ToolActivity["kind"],
) {
  const latestRunning = [...toolActivities.values()]
    .filter(item => item.kind === kind && item.status === "running")
    .sort((a, b) => b.order - a.order);
  return latestRunning[0]?.id;
}

export function thinkingActivity(): AgentActivityItem[] {
  return [
    {
      id: "thinking",
      kind: "thinking",
      status: "running",
    },
  ];
}

function collapseToolActivities(tools: ToolActivity[]): AgentActivityItem[] {
  const items: AgentActivityItem[] = [];
  let index = 0;

  while (index < tools.length) {
    const current = tools[index];
    if (!current)
      break;

    if (current.status === "completed" && (current.kind === "bash" || current.kind === "edit" || current.kind === "write")) {
      const group = [current];
      let cursor = index + 1;
      while (cursor < tools.length) {
        const next = tools[cursor];
        if (!next || next.status !== "completed" || next.kind !== current.kind)
          break;
        group.push(next);
        cursor += 1;
      }

      if (group.length > 1) {
        items.push({
          id: `${current.kind}_${current.order}_group`,
          kind: current.kind,
          status: "completed",
          count: current.kind === "bash" ? group.length : uniqueTargets(group).length || group.length,
        });
        index = cursor;
        continue;
      }
    }

    items.push(toActivityItem(current));
    index += 1;
  }

  return items;
}

function uniqueTargets(group: ToolActivity[]) {
  return [...new Set(group.map(item => item.target).filter(Boolean))];
}

function toActivityItem(tool: ToolActivity): AgentActivityItem {
  return {
    id: tool.id,
    kind: tool.kind,
    status: tool.status,
    target: tool.target,
    detail: tool.detail,
  };
}

function toolFromPayload(payload: unknown, sequence: number): ToolActivity | null {
  if (!isRecord(payload))
    return null;

  const name = stringValue(payload.tool_name)
    ?? stringValue(payload.toolName)
    ?? stringValue(payload.name);
  if (!name || !SUPPORTED_TOOL_NAMES.has(name))
    return null;

  const args = normalizeArgs(payload.tool_args ?? payload.toolArgs ?? payload.arguments);
  const target = targetFromArgs(name as Exclude<AgentActivityKind, "thinking">, args);

  return {
    id: explicitToolId(payload) ?? `${name}_${sequence}`,
    kind: name as Exclude<AgentActivityKind, "thinking">,
    status: "running",
    target: target ? singleLine(target) : undefined,
    detail: target,
    order: sequence,
  };
}

function targetFromToolArgs(
  kind: Exclude<AgentActivityKind, "thinking">,
  argsText: string,
) {
  return targetFromArgs(kind, normalizeArgs(argsText));
}

function targetFromArgs(
  kind: Exclude<AgentActivityKind, "thinking">,
  args: Record<string, unknown> | null,
) {
  if (kind === "bash")
    return stringValue(args?.command);

  return stringValue(args?.path)
    ?? stringValue(args?.file_path)
    ?? stringValue(args?.filePath);
}

function explicitToolId(payload: unknown) {
  if (!isRecord(payload))
    return undefined;

  return stringValue(payload.tool_id)
    ?? stringValue(payload.toolID)
    ?? stringValue(payload.tool_call_id);
}

function normalizeArgs(value: unknown): Record<string, unknown> | null {
  if (isRecord(value))
    return value;
  if (typeof value !== "string")
    return null;

  try {
    const parsed = JSON.parse(value) as unknown;
    return isRecord(parsed) ? parsed : null;
  }
  catch {
    return null;
  }
}

function parseEventPayload(payload?: string | null): unknown {
  if (!payload)
    return null;

  try {
    return JSON.parse(payload) as unknown;
  }
  catch {
    return null;
  }
}

function textFromPayload(payload: unknown) {
  if (!isRecord(payload))
    return "";

  return stringValue(payload.text)
    ?? stringValue(payload.delta)
    ?? stringValue(payload.content)
    ?? "";
}

function hasToolError(payload: unknown) {
  if (!isRecord(payload))
    return false;
  const error = stringValue(payload.error) ?? stringValue(payload.errorText);
  return Boolean(error?.trim());
}

function stringValue(value: unknown) {
  return typeof value === "string" ? value : undefined;
}
