import type { MessageAttachment } from "@future-os/thread-projection";
import { createSendQueueStore, useSendQueueState } from "../../lib/sendQueue";

/**
 * This computer's send queues, one per conversation.
 *
 * Kept beside the conversation views rather than in `AgentThread`'s state so a
 * view that unmounts (switching conversations, reopening the window) does not
 * take the queue with it: the run it waits for is still going on the Agent, and
 * the message belongs to that run.
 */
export const localSendQueue = createSendQueueStore<MessageAttachment>();

/** Subscribe one conversation's view to its queue. */
export function useLocalSendQueue(threadId: string | null) {
  return useSendQueueState(localSendQueue, threadId);
}
