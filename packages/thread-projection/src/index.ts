export type {
  AgentActivityItem,
  AgentActivityKind,
  AgentMessage,
  MessageAttachment,
  MessageRole,
  MessageSegment,
  StreamRetryState,
} from "./model";
export type { RunEvent, SessionEntry } from "./events";
export type {
  AssistantRunProjection,
  RunProjector,
} from "./liveApply";
export type { CollapseRun, ToolKind } from "./group";
export type { ApprovalAction, ApprovalSaveSuggestion } from "./approval";
export type { FriendlyAgentError, RunStatus } from "./format";
export type { CompactionDividerCarrier } from "./compaction";
export { compactionCheckpoints } from "./compaction";

export {
  buildAssistantRunProjection,
  createRunProjector,
  isSoftExit,
  nonZeroExitCode,
} from "./liveApply";
export { entriesToMessages, entriesToTurns, turnsToMessages } from "./projection";
export type { SessionProjectionNode, SessionRunOutcome, SessionTurn } from "./projection";
export {
  asToolKind,
  COLLAPSIBLE_KINDS,
  dedupeByTarget,
  foldCollapsibleRuns,
  isToolKind,
  normalizeArgs,
  targetFromArgs,
} from "./group";
export { classifyAgentError, matchesSettledRun, previousUserMessageBefore } from "./format";
export {
  approvalCommand,
  approvalDeletes,
  approvalPaths,
  escalationTitleKey,
  formatRequestedAction,
  parseAction,
  parseSaveSuggestion,
  unwrapNestedJson,
} from "./approval";
export { isRecord, pathBasename, pathExtension, singleLine, truncate } from "./utils";
export { userMessageFromEvent, upsertUserMessage } from "./userMessage";
export {
  advanceCursor,
  cursorHighWater,
  isPrefixComplete,
  newCursor,
  nextEvent,
} from "./runCursor";
export type { CursorEvent, RunCursor, RunCursorEntry } from "./runCursor";
export {
  normalizeReplayEvent,
  normalizeReplayEvents,
  readRunReplay,
  REPLAY_PAGE_EVENTS,
  tailCoversRange,
} from "./replay";
export type {
  ReplayEvent,
  ReplayEventWire,
  ReplayPage,
  ReplayPageReader,
  ReplayProjection,
  RunReplay,
} from "./replay";

export { HISTORY_PAGE_USER_EXCHANGES, HISTORY_TAIL_CURSOR } from "./historyPage";

export { shellResult } from "./shellResult";
export type { ShellResult, ShellAttempt, ShellStatus } from "./shellResult";
