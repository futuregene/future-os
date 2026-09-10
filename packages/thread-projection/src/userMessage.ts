import type { AgentMessage } from "./model";
import type { SessionEntry } from "./events";
import { entriesToMessages } from "./projection";

/** User events refer to the same persisted entry and run as history pages. */
export function userMessageFromEvent(payload: Record<string, unknown>): AgentMessage | null {
  if (
    typeof payload.entry_id !== "string" ||
    !payload.entry_id ||
    typeof payload.run_id !== "string" ||
    !payload.run_id
  )
    return null;
  const entry: SessionEntry = {
    id: payload.entry_id,
    kind: "user",
    role: "user",
    runId: payload.run_id,
    createdAtMs: typeof payload.created_at_ms === "number" ? payload.created_at_ms : Date.now(),
    blocks: [{ kind: "text", text: typeof payload.text === "string" ? payload.text : "" }],
    metadata: { attachments: Array.isArray(payload.attachments) ? payload.attachments : [] },
  };
  const message = entriesToMessages([entry])[0];
  return message ? { ...message, runId: payload.run_id } : null;
}

/** Identity, never text, distinguishes retries from a new identical prompt. */
export function upsertUserMessage<T extends { id: string; role?: string; runId?: string | null }>(
  messages: T[],
  user: T,
): T[] {
  const index = messages.findIndex(
    message =>
      message.role === "user" &&
      (message.id === user.id || (!!user.runId && message.runId === user.runId)),
  );
  if (index >= 0) {
    const next = [...messages];
    next[index] = { ...messages[index]!, ...user, id: messages[index]!.id };
    return next;
  }
  // The assistant can arrive first through a different subscription.
  const assistant = user.runId
    ? messages.findIndex(message => message.role === "assistant" && message.runId === user.runId)
    : -1;
  const at = assistant < 0 ? messages.length : assistant;
  return [...messages.slice(0, at), user, ...messages.slice(at)];
}
