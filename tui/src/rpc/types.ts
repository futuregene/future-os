/**
 * RPC types for FutureAgent communication.
 * Mirrors the Rust rpc module on the server side.
 */

// ============================================================================
// RPC Command (matches Go RpcCommand - all fields on one struct)
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
  // Session
  sessionPath?: string;
  sessionId?: string;
  entryId?: string;
  name?: string;
  outputPath?: string;
  runId?: string;
  sinceIdx?: number;
  requestedRunId?: string;
  clientRequestId?: string;
  busyPolicy?: "reject_if_busy" | "enqueue_if_busy" | "supersede_session";
}

// ============================================================================
// Specific command creators (for type safety at call sites)
// ============================================================================

export function promptCmd(message: string, images?: ImageContent[], busyPolicy: RpcCommand["busyPolicy"] = "reject_if_busy"): RpcCommand {
  return { type: "prompt", message, images, busyPolicy };
}

// ============================================================================
// Types
// ============================================================================

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";

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
// RPC State
// ============================================================================

export interface RpcSessionState {
  agentInstanceId?: string;
  model?: string;
  thinkingLevel: ThinkingLevel;
  isStreaming: boolean;
  isCompacting: boolean;
  sessionFile?: string;
  sessionId: string;
  session_name?: string;
  explicitSession: boolean;
  autoCompactionEnabled: boolean;
  queryCount: number;
  // Welcome info
  version?: string;
  cwd?: string;
  permissionLevel?: "all" | "workspace" | "none";
  skills?: string[];
  contextFiles?: string[];
  extensions?: string[];
  // Context usage
  contextTokens?: number;
  contextWindow?: number;
  contextPercent?: number;
  // Token usage (cumulative for session)
  tokensIn?: number;
  tokensOut?: number;
  tokensCacheR?: number;
  tokensCacheW?: number;
  totalCost?: number;
  activeRun?: ActiveRunState | null;
  queuedRuns?: QueuedRunState[];
  queuedCount?: number;
  interruptedRun?: InterruptedRunState | null;
  requestedRun?: RunTerminalState | null;
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
  state: "starting" | "running" | "cancelling" | "cancellation_stuck" | "persistence_degraded" | "finalizing";
  lastEventIdx: number;
}

/// A run that began (durable run_started marker) but never committed — recovered
/// as interrupted after an agent crash/restart. Reported by get_state when no
/// run is live but the session journal has an unterminated run.
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
// Session Summary (from list_sessions)
// ============================================================================

export interface SessionSummary {
  id: string;
  cwd: string;
  updated_at: string;
  model: string;
  session_name?: string;
  parent_session_id?: string;
  is_streaming?: boolean;
}

// ============================================================================
// Model Info (from get_available_models)
// ============================================================================

export interface ModelInfo {
  id: string;
  label: string;       // display name (was "name")
  provider: string;
  supportsImages: boolean;  // was "image"
  thinkingLevel: string;    // default thinking level for this model
  contextWindow: number;
  isDefault: boolean;
}

// ============================================================================
// Agent Events
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
  text?: string;   // text_chunk, agent_end, tool_delta
  tool_id?: string;
  tool_name?: string;
  [key: string]: unknown;
};

export interface ProjectedRunEvent {
  type: string;
  data: string;
  idx: number;
}
