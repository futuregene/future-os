import type { AgentMessage } from "@future-os/thread-projection";
import { upsertUserMessage } from "@future-os/thread-projection";

/** UI ids can be optimistic; a run binds the persisted and live exchange. */
export function historyMessageKey(message: AgentMessage): string {
  return message.runId ? `${message.role}:${message.runId}` : message.id;
}

/** Reconcile a tail snapshot without deleting newer writes or remounting rows. */
export function reconcileThreadHistory(
  current: AgentMessage[],
  incoming: AgentMessage[],
  atRequest: AgentMessage[],
  keepOlder: boolean,
  settle: boolean,
): { messages: AgentMessage[]; keptOlder: boolean } {
  if (!incoming.length && current.length)
    return { messages: current, keptOlder: true };
  const existing = new Map(current.map(message => [historyMessageKey(message), message]));
  const baseline = new Map(atRequest.map(message => [historyMessageKey(message), message]));
  const incomingKeys = new Set(incoming.map(historyMessageKey));
  const first = incoming[0];
  const anchor = keepOlder && first ? current.findIndex(message => historyMessageKey(message) === historyMessageKey(first)) : -1;
  const prefix = anchor > 0 ? current.slice(0, anchor) : [];
  const prefixKeys = new Set(prefix.map(historyMessageKey));
  const tail = incoming.map((message) => {
    const key = historyMessageKey(message);
    const previous = existing.get(key);
    if (!previous)
      return message;
    if (previous !== baseline.get(key) || (previous.status === "streaming" && !settle))
      return previous;
    const merged = { ...message, id: previous.id, attachments: message.attachments ?? previous.attachments };
    const fields = new Set([...Object.keys(previous), ...Object.keys(merged)]) as Set<keyof AgentMessage>;
    return [...fields].every(field => previous[field] === merged[field]) ? previous : merged;
  });
  const pending = current.filter((message) => {
    const key = historyMessageKey(message);
    return !incomingKeys.has(key) && !prefixKeys.has(key)
      && (message !== baseline.get(key) || message.status === "streaming"
        || message.id.startsWith("pending") || message.id.startsWith("local_assistant"));
  });
  const placed = [...prefix, ...tail];
  // A prompt the page does not hold goes above the reply that answers it, and
  // the clock cannot say where that is: a live bubble is stamped with its run's
  // start, which precedes the prompt's own entry time by a few milliseconds, so
  // a prompt that arrived while this reload was in flight landed BELOW the empty
  // answer being written for it (prompt sent from the phone, read on the
  // desktop). The run id can — it is the same pairing `upsertUserMessage` uses
  // when the prompt arrives as an event instead.
  const pendingPrompts: AgentMessage[] = [];
  const pendingRest: AgentMessage[] = [];
  for (const message of pending) {
    if (message.role === "user" && message.runId)
      pendingPrompts.push(message);
    else
      pendingRest.push(message);
  }
  // Prompts go in first: a reply the page already carries is what places them,
  // and an optimistic bubble of the same local exchange must stay *below* its
  // prompt (a reply above a nameless reply reads as that reply's snapshot).
  let messages = placed;
  for (const prompt of pendingPrompts) {
    // Only a prompt whose reply is already on screen can be placed by identity;
    // one whose run never reached the agent still belongs at its own time.
    const answered = messages.some(existing => existing.role === "assistant"
      && (existing.runId === prompt.runId || existing.inFlightRunId === prompt.runId));
    messages = answered ? upsertUserMessage(messages, prompt) : mergeLocalOnly(messages, [prompt]);
  }
  messages = mergeLocalOnly(messages, pendingRest);
  return {
    messages: messages.length === current.length && messages.every((message, index) => message === current[index]) ? current : messages,
    keptOlder: prefix.length > 0,
  };
}

/**
 * Merge rows the history page does not contain at their own place in time.
 *
 * Such a row is usually the newest write — the optimistic bubble of a send still
 * in flight — and belongs last. But it can also be an OLD one: a send whose
 * prompt never reached the agent (network/IPC failure) is persisted nowhere, so
 * its bubble stays local-only for good, and appending it on every reload moved
 * an exchange from hours ago below the newest reply — the transcript then read
 * newest-first. Insert by `createdAt` instead, stably, so an undated row still
 * lands last and rows that share a timestamp keep their order.
 */
function mergeLocalOnly(
  messages: AgentMessage[],
  localOnly: AgentMessage[],
): AgentMessage[] {
  if (!localOnly.length)
    return messages;
  const out = [...messages];
  for (const message of localOnly) {
    const at = Date.parse(message.createdAt);
    const index = Number.isFinite(at)
      ? out.findIndex((existing) => {
          const other = Date.parse(existing.createdAt);
          return Number.isFinite(other) && other > at;
        })
      : -1;
    if (index === -1)
      out.push(message);
    else
      out.splice(index, 0, message);
  }
  return out;
}
