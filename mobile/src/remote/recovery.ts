import type { TimelineItem } from "./types";

/** Desktop-parity recovery predicate: failed latest run, never a user stop. */
export function canRecoverMessage(item: TimelineItem, latestAssistantId: string | null): boolean {
  return (
    item.kind === "message" &&
    item.role === "assistant" &&
    item.failed === true &&
    item.id === latestAssistantId &&
    item.stopped !== true &&
    typeof item.runId === "string" &&
    item.runId.length > 0
  );
}
