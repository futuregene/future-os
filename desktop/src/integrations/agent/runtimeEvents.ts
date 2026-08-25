export interface ThreadRuntimeUpdate {
  threadId: string;
  runId: string;
  revision: number;
  status: string;
  resetProjection: boolean;
}

/** One frame's coalesced run invalidations from the desktop backend. */
export interface ThreadRuntimeUpdateBatch {
  updates: ThreadRuntimeUpdate[];
}
