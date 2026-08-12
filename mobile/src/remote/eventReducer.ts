// Barrel over the mobile projection layer. The projection semantics (live event
// folding, history projection) live in the shared @future-os/thread-projection
// package; this module re-exports the mobile mapping layer and the UI-shell
// state (TimelineState) that RemoteContext consumes. Kept under the historical
// name so RemoteContext's imports and the pinned tests stay stable.
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
} from "./projection";
export type { ReplayEventWire } from "./projection";
export type { TimelineState } from "./projection";
