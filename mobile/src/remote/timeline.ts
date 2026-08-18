// Barrel over the mobile projection layer. The projection semantics (live event
// folding, history projection) live in the shared @future-os/thread-projection
// package; this module re-exports the mobile mapping layer and the UI-shell
// state (TimelineState) that RemoteContext consumes.
export {
  appendUserMessage,
  applyStreamEvent,
  emptyTimeline,
  markApprovalDecision,
  mergeHistoryAttachments,
  normalizeReplayEvents,
  stripRunItems,
  timelineFromEntries,
  timelineFromHistory,
  timelineFromProjection,
  upsertTruncationNotice,
} from "./projection";
export type { ReplayEventWire } from "./projection";
export type { TimelineState } from "./projection";
