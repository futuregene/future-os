import type { MobileAttachment } from "../../remote/types";
import { createSendQueueStore, useSendQueueState } from "../../lib/sendQueue";

/**
 * This phone's send queues, one per remote conversation.
 *
 * Keyed the way the conversation is (`desktop:session`), because every
 * conversation on this phone lives on a paired desktop: the same session id on
 * two machines is two different conversations, and a message queued for one must
 * never leave from the other.
 */
export const sendQueue = createSendQueueStore<MobileAttachment>();

/** Subscribe one conversation's view to its queue. */
export function useSendQueue(conversationKey: string | null) {
  return useSendQueueState(sendQueue, conversationKey);
}
