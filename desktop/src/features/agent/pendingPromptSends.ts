/**
 * Staged first prompts a thread view has picked up and is sending.
 *
 * A staged prompt changes hands in two steps — the thread view latches it, then
 * the Agent accepts it — and between them the prompt is still staged while no
 * longer the *caller's* to reclaim. The holder lives at module scope, not in a
 * ref, so it also survives the thread view unmounting mid-send (attachment
 * preparation can unmount it before the send settles); an instance-local ref
 * alone permits a second submission of the same prompt.
 *
 * Shares the set with {@link isPromptSendInFlight}, which the new-conversation
 * flow asks before it hands an undelivered prompt back to its draft: a prompt a
 * send owns must not also come back as an unsent draft, or the same message
 * would go out twice.
 */
const inFlight = new Set<string>();

export function markPromptSendInFlight(id: string): void {
  inFlight.add(id);
}

export function clearPromptSendInFlight(id: string): void {
  inFlight.delete(id);
}

export function isPromptSendInFlight(id: string): boolean {
  return inFlight.has(id);
}
