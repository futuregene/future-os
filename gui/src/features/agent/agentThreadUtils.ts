import type { StoredMessage, StoredRun } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./types";
import {
  storedTimeToIso,
  updateRunStatus,
} from "../../integrations/storage/threadStore";
import { parseMessageContent } from "./attachments";

export function matchesSettledRun(status: StoredRun["status"]) {
  return status === "completed" || status === "failed" || status === "cancelled";
}

export function toAgentMessage(message: StoredMessage): AgentMessage {
  const content = parseMessageContent(message.content, message.contentType);

  return {
    id: message.id,
    runId: message.runId,
    role: message.role === "user" ? "user" : "assistant",
    author: message.role === "user" ? "You" : "Research Copilot",
    content: content.text,
    status: message.status,
    createdAt: storedTimeToIso(message.createdAt),
    attachments: content.attachments,
  };
}

export function buildAgentFailureContent(message: string) {
  return `Future Agent 连接失败：${message}\n\n请确认 agent 已启动，并且 FUTURE_AGENT_GRPC_ADDR 指向 127.0.0.1:50051。`;
}

export async function updateRunStatusSafe(
  runId: string,
  status: "completed" | "failed",
  errorMessage?: string,
) {
  try {
    await updateRunStatus({ runId, status, errorMessage });
  }
  catch {
    // Run status persistence is best-effort; the visible assistant message
    // still records the failure for the user.
  }
}
