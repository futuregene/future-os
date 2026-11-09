import type { AgentMessage } from "./model";
import { collapseReplyDuplicates } from "./replyDuplicates";
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
  const entry: import("./events").SessionEntry = {
    id: payload.entry_id,
    kind: "user",
    role: "user",
    runId: payload.run_id,
    createdAtMs: typeof payload.created_at_ms === "number" ? payload.created_at_ms : Date.now(),
    blocks: [{ kind: "text", text: typeof payload.text === "string" ? payload.text : "" }],
    metadata: { attachments: Array.isArray(payload.attachments) ? payload.attachments : [] },
  };
  const message = entriesToMessages([entry])[0];
  return message
    ? { ...message, sourceEntryId: payload.entry_id, runId: payload.run_id }
    : null;
}

/** Identity, never text, distinguishes retries from a new identical prompt. */
export function upsertUserMessage<
  T extends { id: string; role?: string; runId?: string | null; inFlightRunId?: string | null },
>(messages: T[], user: T): T[] {
  // The turn's reply is drawn once before anything is placed: a reload during
  // the run appends a snapshot of the answer beside the live bubble, and a prompt
  // landing in an array like that is put between two halves of its own answer.
  const base = collapseReplyDuplicates(messages);
  const index = base.findIndex(
    message =>
      message.role === "user" &&
      (message.id === user.id || (!!user.runId && message.runId === user.runId)),
  );
  if (index >= 0) {
    const next = [...base];
    next[index] = { ...base[index]!, ...user, id: base[index]!.id };
    return next;
  }
  // The assistant can arrive first through a different subscription — and when
  // this conversation was reloaded mid-run, that reply carries its run in
  // `inFlightRunId` instead of `runId` (see `AgentMessage.inFlightRunId`): the
  // stamp is withheld so the live-bubble guards keep drawing the growing reply,
  // while the turn's identity survives for this pairing. Without it a prompt
  // landed *below* its own answer, and the wrong order outlived a thread switch
  // because the warm snapshot kept it.
  const assistant = user.runId
    ? base.findIndex(message =>
        message.role === "assistant"
        && (message.runId === user.runId || message.inFlightRunId === user.runId))
    : -1;
  const at = assistant < 0 ? base.length : assistant;
  return [...base.slice(0, at), user, ...base.slice(at)];
}
