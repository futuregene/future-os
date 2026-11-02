import type { RemoteUpload } from "./remotePeerClient";
import { createSendQueueStore, useSendQueueState } from "../../lib/sendQueue";

/**
 * The send queues of the computers this app drives, one per remote conversation.
 *
 * Keyed by desktop *and* session: the same session id on two machines is two
 * different conversations, and a message queued for one must never leave from
 * the other.
 */
export const remoteSendQueue = createSendQueueStore<RemoteUpload>();

export function remoteSendQueueKey(desktopId: string, sessionId: string): string {
  return `${desktopId}:${sessionId}`;
}

/** Subscribe one remote conversation's view to its queue. */
export function useRemoteSendQueue(desktopId: string, sessionId: string) {
  return useSendQueueState(remoteSendQueue, remoteSendQueueKey(desktopId, sessionId));
}
