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
