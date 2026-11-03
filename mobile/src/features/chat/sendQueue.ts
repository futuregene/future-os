import type { MobileAttachment } from "../../remote/types";
import { createSendQueueStore, useSendQueueState, type QueuedSend } from "../../lib/sendQueue";
import { loadSendQueue, saveSendQueue } from "./sendQueuePersistence";

/**
 * This phone's send queues, one per remote conversation.
 *
 * Keyed the way the conversation is (`desktop:session`), because every
 * conversation on this phone lives on a paired desktop: the same session id on
 * two machines is two different conversations, and a message queued for one must
 * never leave from the other.
 *
 * Outlives the process: every change is written to AsyncStorage (`sendQueuePersistence`),
 * and what was there is loaded back when the app starts. A phone that is rebooted,
 * or whose app was reclaimed in the background, does not lose a sentence the reader
 * had already committed to sending.
 */
const persisted = new Map<string, readonly QueuedSend<MobileAttachment>[]>();

export const sendQueue = createSendQueueStore<MobileAttachment>({
  onChange: (key, state) => {
    if (state.items.length === 0)
      persisted.delete(key);
    else
      persisted.set(key, state.items);
    void saveSendQueue(persisted);
  },
});

/**
 * Hydrate from storage, once, when the module is first used.
 *
 * Not awaited by the store's consumers: the map fills in asynchronously and the
 * subscribers re-render, which is how the panel shows a queue that was waiting
 * for the app to come back. A queue already live for a conversation wins
 * (`SendQueueStore.hydrate`), so typing during the read is never overwritten.
 */
void (async () => {
  const stored = await loadSendQueue();
  for (const [key, items] of stored)
    sendQueue.hydrate(key, items);
})();

/** Subscribe one conversation's view to its queue. */
export function useSendQueue(conversationKey: string | null) {
  return useSendQueueState(sendQueue, conversationKey);
}
