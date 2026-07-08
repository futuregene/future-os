import type { StoredRunEvent, StoredToolCall, StoredToolOutput } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import { listRunEvents, listToolCalls, listToolOutputs } from "../../integrations/storage/threadStore";
import { isRecord, truncate } from "../../lib/objects";
import { unwrapNestedJson } from "./approvalPayload";

/**
 * Prompt construction for the "continue / retry a run" recovery flows. Kept out
 * of the AgentThread view component so it stays focused on composition.
 */
export function buildContinuePrompt({
  message,
  runId,
  summary,
}: {
  message?: AgentMessage;
  runId?: string;
  summary?: string;
}) {
  const effectiveRunId = runId ?? message?.runId ?? null;
  const lines = [
    "继续上一个任务。",
    "请基于当前线程、最近一次运行状态和工作区当前文件状态继续推进。",
    "不要重复执行已经成功完成的副作用操作；如果需要再次写入、删除、执行复杂命令，请先说明原因并遵守审批策略。",
  ];

  if (effectiveRunId) {
    lines.push("", `最近 Run: ${effectiveRunId}`);
  }
  if (message?.content?.trim()) {
    lines.push("", "上一条失败消息摘要:", truncate(message.content.trim(), 1200));
  }
  if (summary?.trim()) {
    lines.push("", "已执行内容摘要:", summary.trim());
  }

  return lines.join("\n");
}

export async function loadRunResumeSummary(runId: string) {
  try {
    const [events, tools] = await Promise.all([
      listRunEvents(runId),
      listToolCalls(runId),
    ]);
    const outputEntries = await Promise.all(
      tools.slice(0, 8).map(async (tool) => {
        try {
          return [tool.id, await listToolOutputs(tool.id)] as const;
        }
        catch {
          return [tool.id, [] as StoredToolOutput[]] as const;
        }
      }),
    );
    const outputsByTool = Object.fromEntries(outputEntries);
    return summarizeRunForPrompt(events, tools, outputsByTool);
  }
  catch (error) {
    return `Run 摘要加载失败：${error instanceof Error ? error.message : String(error)}`;
  }
}

export function previousUserForRun(messages: AgentMessage[], runId: string) {
  const runMessageIndex = messages.findIndex(message => message.runId === runId && message.role === "assistant");
  const startIndex = runMessageIndex >= 0 ? runMessageIndex - 1 : messages.length - 1;
  for (let index = startIndex; index >= 0; index -= 1) {
    const message = messages[index];
    if (message?.role === "user") {
      return message;
    }
  }
  return null;
}

function summarizeRunForPrompt(
  events: StoredRunEvent[],
  tools: StoredToolCall[],
  outputsByTool: Record<string, StoredToolOutput[]>,
) {
  const lines: string[] = [];
  if (tools.length > 0) {
    lines.push("工具调用:");
    for (const tool of tools.slice(0, 8)) {
      const command = toolCommand(tool.input) ?? tool.input ?? tool.name;
      const outputs = outputsByTool[tool.id] ?? [];
      const outputSummary = outputs
        .map(output => output.content ?? output.kind)
        .filter(Boolean)
        .map(value => truncate(value, 240))
        .join(" | ");
      lines.push(`- ${tool.name} [${tool.status}]: ${truncate(command, 360)}${outputSummary ? ` => ${outputSummary}` : ""}`);
    }
    if (tools.length > 8) {
      lines.push(`- 还有 ${tools.length - 8} 个工具调用未展开。`);
    }
  }

  const finalEvents = events
    .filter(event => ["error", "agent_error", "agent_end", "tool_end", "tool_result"].includes(event.eventType))
    .slice(-6);
  if (finalEvents.length > 0) {
    lines.push("最近事件:");
    for (const event of finalEvents) {
      lines.push(`- ${event.eventType}: ${truncate(event.payload ?? "", 360)}`);
    }
  }

  return lines.join("\n");
}

function toolCommand(input: string | null | undefined) {
  if (!input)
    return null;

  let parsed: unknown;
  try {
    parsed = unwrapNestedJson(input);
  }
  catch {
    return null;
  }
  if (isRecord(parsed) && typeof parsed.command === "string" && parsed.command.trim())
    return parsed.command;
  return null;
}
