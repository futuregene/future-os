/**
 * Shared RPC payload types for the TS clients (TUI / CLI).
 * Merged superset of the former tui/cli `rpc/types.ts`; mirrors the Rust
 * `future_rpc` payload structs and `future-rpc/proto/future.proto`.
 */

// ============================================================================
// RPC Command (matches proto RpcCommand — union of all command fields)
// ============================================================================

export interface RpcCommand {
  id?: string;
  type: string;
  // prompting
  message?: string;
  images?: ImageContent[];
  // new_session
  parentSession?: string;
  cwd?: string;
  createdBy?: string;
  sourceMeta?: string;
  // set_model
  provider?: string;
  modelId?: string;
  // set_thinking_level
  level?: ThinkingLevel;
  mode?: string;
  // compact
  customInstructions?: string;
  // set_auto_compaction / set_auto_retry
  enabled?: boolean;
  // set_enabled_models
  enabledModels?: string[];
  // shell
  command?: string;
  // session bookkeeping
  sessionPath?: string;
  sessionId?: string;
  entryId?: string;
  name?: string;
  outputPath?: string;
  // set_system_prompt / append_system_prompt
  systemPrompt?: string;
  // set_tools / disable_tools / disable_builtin_tools
  tools?: string[];
  noTools?: boolean;
  // set_ephemeral
  ephemeral?: boolean;
  // runs / replay
  runId?: string;
  sinceIdx?: number;
  requestedRunId?: string;
  clientRequestId?: string;
  busyPolicy?: "reject_if_busy" | "enqueue_if_busy" | "supersede_session";
}

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";

export type PermissionLevel = "all" | "workspace" | "none";

export interface ImageContent {
  type: "image";
  source: {
    type: "base64" | "url";
    media_type: string;
    data?: string;
    url?: string;
  };
}

// ============================================================================
// RPC Responses
// ============================================================================

export interface RpcResponse {
  id?: string;
  type: "response";
  command: string;
  success: boolean;
  data?: unknown;
  /** Typed payload oneof (typed-RPC migration); absent on old agents. */
  payload?: unknown;
  error?: string;
  errorCode?: string;
  errorData?: unknown;
}

export interface RunAck {
  run_id: string;
  run_epoch: number;
  accepted_state: "existing" | "running" | "queued";
  run_sequence?: number;
  queue_position?: number;
}

// ============================================================================
// Session state (get_state)
// ============================================================================

export interface RpcSessionState {
  agentInstanceId?: string;
  model?: string;
  thinkingLevel: ThinkingLevel;
  isStreaming: boolean;
  isCompacting: boolean;
  sessionFile?: string;
  sessionId: string;
  sessionName?: string;
  /** Legacy alias of `sessionName` emitted during the migration window. */
  session_name?: string;
  explicitSession: boolean;
  autoCompactionEnabled: boolean;
  queryCount: number;
  version?: string;
  cwd?: string;
  permissionLevel?: "all" | "workspace" | "none";
  skills?: string[];
  contextFiles?: string[];
  extensions?: string[];
  contextTokens?: number;
  contextWindow?: number;
  contextPercent?: number;
  tokensIn?: number;
  tokensOut?: number;
  tokensCacheR?: number;
  tokensCacheW?: number;
  totalCost?: number;
  activeRun?: ActiveRunState | null;
  queuedRuns?: QueuedRunState[];
  queuedCount?: number;
  recentTerminalAcks?: RecentTerminalAck[];
  interruptedRun?: InterruptedRunState | null;
  requestedRun?: RunTerminalState | null;
}

export interface RecentTerminalAck {
  runId: string;
  runSequence: number;
  clientRequestId: string;
  state: "terminal" | "cancelled" | "failed";
  reason: string;
  /** Legacy snake_case aliases, always emitted during the migration window. */
  run_id: string;
  run_sequence: number;
  client_request_id: string;
}

export interface QueuedRunState {
  runId: string;
  runSequence: number;
  clientRequestId: string;
  state: "queued";
  queuePosition: number;
  acceptedAt: string;
  displayText: string;
}

export interface ActiveRunState {
  runId: string;
  epoch: number;
  runSequence?: number;
  state:
    | "starting"
    | "running"
    | "cancelling"
    | "cancellation_stuck"
    | "persistence_degraded"
    | "finalizing";
  lastEventIdx: number;
}

export interface InterruptedRunState {
  runId: string;
  state: "interrupted_by_restart";
}

export interface RunTerminalState {
  run_id: string;
  state: "completed" | "error" | "cancelled" | "incomplete" | "interrupted_by_restart";
  run_tokens: number;
  run_duration_ms: number;
  error?: string;
}

// ============================================================================
// Session summary (list_sessions)
// ============================================================================

export interface SessionSummary {
  id: string;
  cwd: string;
  model: string;
  updatedAt: string;
  /** Legacy snake_case alias; always emitted during the migration window. */
  updated_at: string;
  sessionName?: string;
  firstMessage?: string;
  queryCount?: number;
  isStreaming?: boolean;
  parentSessionId?: string;
  /** Legacy snake_case aliases emitted during the migration window. */
  session_name?: string;
  first_message?: string;
  query_count?: number;
  is_streaming?: boolean;
  parent_session_id?: string;
}

// ============================================================================
// Model info (list_models)
// ============================================================================

export interface ModelInfo {
  id: string;
  label: string;
  provider: string;
  supportsImages: boolean;
  thinkingLevel: string;
  contextWindow: number;
  isDefault: boolean;
}

// ============================================================================
// Agent events (StreamEvent)
// ============================================================================

export type AgentEvent = {
  type: string;
  sessionId?: string;
  runId?: string;
  epoch?: number;
  idx?: number;
  eventId?: string;
  timestamp?: string;
  projectionSnapshot?: boolean;
  snapshotCursor?: number;
  snapshotEvents?: ProjectedRunEvent[];
  /** Typed payload oneof (typed-RPC migration); absent on old agents. */
  payload?: unknown;
  text?: string;
  tool_id?: string;
  tool_name?: string;
  [key: string]: unknown;
};

export interface ProjectedRunEvent {
  type: string;
  data: string;
  idx: number;
  /** Typed payload oneof (typed-RPC migration); absent on old agents. */
  payload?: unknown;
}
