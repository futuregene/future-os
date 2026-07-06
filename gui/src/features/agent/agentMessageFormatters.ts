import type { StoredMessage, StoredRun } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import i18n from "../../i18n";
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
    author: message.role === "user" ? i18n.t("agent:author.you") : i18n.t("agent:author.researchCopilot"),
    authorKey: message.role === "user" ? "author.you" : "author.researchCopilot",
    content: content.text,
    status: message.status,
    createdAt: storedTimeToIso(message.createdAt),
    attachments: content.attachments,
  };
}

export function buildAgentFailureContent(message: string) {
  // Only a genuine gRPC connection failure (prefixed by the Tauri bridge)
  // warrants the "check the agent is running" guidance. Other errors — e.g. the
  // model API rejecting the request (quota / tenant permission) — are run
  // failures, not connectivity problems, and mislabeling them as "connection failure" sends
  // users to debug the wrong thing.
  if (message.includes("Unable to connect to Future Agent")) {
    return i18n.t("agent:failure.connect", { message });
  }
  return i18n.t("agent:failure.run", { message });
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
