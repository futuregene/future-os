import type { AgentMessage } from "@future-os/thread-projection";
import { classifyAgentError } from "@future-os/thread-projection";
import i18n from "../../i18n";
import { updateRunStatus } from "../../integrations/storage/threadStore";

export { classifyAgentError, matchesSettledRun, previousUserMessageBefore } from "@future-os/thread-projection";
export type { FriendlyAgentError } from "@future-os/thread-projection";

/** Translate a raw agent/run error into the user-facing failure text. */
export function friendlyAgentError(raw: string): string {
  const { key, params } = classifyAgentError(raw);
  return i18n.t(key, params);
}

export function buildAgentFailureContent(message: string) {
  return friendlyAgentError(message);
}

/** Short state for the divider; the accompanying content contains only the next step. */
export function buildAgentFailureTitle(message: string) {
  const { key, params } = classifyAgentError(message);
  const titleKey = `${key}Title`;
  const title = i18n.t(titleKey, params);
  return title === titleKey ? i18n.t("agent:failure.runTitle") : title;
}

/** Explain an explicit user stop without presenting it as a failure. */
export function userStoppedNotice(): string {
  return i18n.t("agent:failure.userStopped");
}

export function responseTerminationError(kind?: string | null): string {
  const codes: Record<string, string> = {
    upstream_disconnected: "UPSTREAM_DISCONNECTED",
    response_timeout: "RESPONSE_TIMEOUT",
    output_limit: "OUTPUT_LIMIT",
    model_content_filter: "MODEL_CONTENT_FILTER",
    model_response_error: "MODEL_RESPONSE_ERROR",
    model_paused: "MODEL_PAUSED",
    provider_cancelled: "PROVIDER_CANCELLED",
  };
  return `[${codes[kind ?? ""] ?? "RESPONSE_UNCONFIRMED"}] response did not complete`;
}

/** Empty text is a presentation outcome, not proof of a protocol failure. */
export function isCompletedWithoutReply(message: AgentMessage): boolean {
  return message.role === "assistant"
    && message.status === "complete"
    && !message.stopped
    && !message.terminationNotice
    && !message.content.trim()
    && !message.segments?.some(segment => segment.kind === "activity"
      || (segment.kind === "text" && segment.text.trim()))
    && !message.activityItems?.some(item => item.kind !== "thinking");
}

export function canContinueResponse(message: AgentMessage): boolean {
  const { key } = classifyAgentError(message.runError ?? "");
  return !["agent:failure.modelPaused", "agent:failure.contentFilter", "agent:failure.softwareError"].includes(key);
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
