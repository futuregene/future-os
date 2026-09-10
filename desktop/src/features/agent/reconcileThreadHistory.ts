import type { AgentMessage } from "@future-os/thread-projection";

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
  const messages = [...prefix, ...tail, ...pending];
  return {
    messages: messages.length === current.length && messages.every((message, index) => message === current[index]) ? current : messages,
    keptOlder: prefix.length > 0,
  };
}
