import type { AgentMessage } from "./model";
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
  // The assistant can arrive first through a different subscription — and when
  // this conversation was reloaded mid-run, that reply carries its run in
  // `inFlightRunId` instead of `runId` (see `AgentMessage.inFlightRunId`): the
  // stamp is withheld so the live-bubble guards keep drawing the growing reply,
  // while the turn's identity survives for this pairing. Without it a prompt
  // landed *below* its own answer, and the wrong order outlived a thread switch
  // because the warm snapshot kept it.
  const assistant = user.runId
    ? messages.findIndex(message =>
        message.role === "assistant"
        && (message.runId === user.runId || message.inFlightRunId === user.runId))
    : -1;
  if (assistant < 0)
    return [...messages, user];

  // A reply can be on screen twice for one turn. A reload during the run
  // projects the agent's mid-run snapshot into the array as a settled-looking
  // reply carrying no run of its own, and the live bubble for the same run is
  // appended beside it. The prompt then landed between the two halves of its own
  // answer — measured as "7s reply" / question / "2s reply" in one case, where a
  // reload from the journal rendered it correctly: the pair on disk is fine, only
  // this array was wrong.
  //
  // The fragments directly above the matched reply are that snapshot: nothing
  // but replies sits between them and the reply (they are contiguous), and they
  // carry no run because its stamp is withheld while the run is in flight. They
  // go, because with the prompt above the real reply they can only render as a
  // second, stale copy of the same answer.
  let at = assistant;
  while (at > 0) {
    const above = messages[at - 1];
    if (!above || !isReplyFragment(above)) break;
    at -= 1;
  }
  const head = [...messages.slice(0, at), user];
  // Everything from `at` on, minus the swallowed fragments.
  for (let index = at; index < messages.length; index += 1) {
    const message = messages[index];
    if (!message || index >= assistant || !isReplyFragment(message))
      head.push(message as T);
  }
  return head;
}

/**
 * Whether a message is a snapshot of a reply whose run this client cannot name.
 *
 * That is the shape a mid-run reload leaves: an assistant that renders as
 * settled but carries no run, because `applyRunMetadata` withholds the stamp so
 * the live bubble keeps ownership of the reply. A live bubble is never one of
 * these — it always carries the run it draws — so this cannot match the reply
 * that is actually growing.
 */
function isReplyFragment(
  message: { role?: string; runId?: string | null; inFlightRunId?: string | null },
): boolean {
  return message.role === "assistant" && !message.runId && !message.inFlightRunId;
}
