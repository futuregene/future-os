export interface ThreadRuntimeUpdate {
  threadId: string;
  runId: string;
  revision: number;
  status: string;
  resetProjection: boolean;
}

/**
 * One conversation's activity stamp advanced on the backend.
 *
 * Carries the value the store wrote, so the sidebar patches that row's
 * `lastMessageAt` instead of reloading the list: the age label and the row's
 * place both derive from it (see `applyThreadActivity`).
 */
export interface ThreadActivityUpdate {
  threadId: string;
  /** Unix millis, as written to `threads.last_message_at`. */
  lastMessageAt: number;
}

/** One frame's coalesced run invalidations from the desktop backend. */
export interface ThreadRuntimeUpdateBatch {
  updates: ThreadRuntimeUpdate[];
}
