import type { StoredRunEvent } from "../../integrations/storage/threadStore";
import type { AgentActivityItem, AgentActivityKind } from "./agentThreadTypes";

interface AssistantRunProjection {
  activityItems: AgentActivityItem[];
  content: string;
}

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
  let content = "";
  let thinking = false;
  let sawVisibleWork = false;
  let activeToolCallId: string | null = null;

  for (const event of sortedEvents) {
    const payload = parseEventPayload(event.payload);

    if (event.eventType === "text_chunk") {
      const text = textFromPayload(payload);
      content += text;
      if (text.trim()) {
        sawVisibleWork = true;
      }
      continue;
    }

    if (event.eventType === "thinking_start") {
      thinking = true;
      continue;
    }

    if (event.eventType === "thinking_end") {
      thinking = false;
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
              target: compactTarget(target),
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
      sawVisibleWork = true;
    }
  }

  const items = collapseToolActivities([...toolActivities.values()].sort((a, b) => a.order - b.order));
  if (thinking && !content.trim() && !sawVisibleWork) {
    items.unshift({
      id: "thinking",
      kind: "thinking",
      status: "running",
    });
  }

  return {
    activityItems: items,
    content,
  };
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

    if (current.status === "completed" && (current.kind === "bash" || current.kind === "edit" || current.kind === "write")) {
      const group = [current];
      let cursor = index + 1;
      while (
        cursor < tools.length
        && tools[cursor].status === "completed"
        && tools[cursor].kind === current.kind
      ) {
        group.push(tools[cursor]);
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
    target: target ? compactTarget(target) : undefined,
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

function compactTarget(value: string) {
  return value.replace(/\s+/g, " ").trim();
}

function stringValue(value: unknown) {
  return typeof value === "string" ? value : undefined;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
