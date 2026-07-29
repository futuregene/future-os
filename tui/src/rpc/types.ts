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
  streamingBehavior?: "steer" | "followUp";
  // new_session
  parentSession?: string;
  cwd?: string;
  // set_model
  provider?: string;
  modelId?: string;
  // set_thinking_level
  level?: ThinkingLevel;
  // set_steering_mode / set_follow_up_mode
  mode?: "all" | "one-at-a-time";
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
}

// ============================================================================
// Specific command creators (for type safety at call sites)
// ============================================================================

export function promptCmd(message: string, images?: ImageContent[], streamingBehavior?: "steer" | "followUp"): RpcCommand {
  return { type: "prompt", message, images, streamingBehavior };
}
export function steerCmd(message: string): RpcCommand {
  return { type: "steer", message };
}
export function followUpCmd(message: string): RpcCommand {
  return { type: "follow_up", message };
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
}

// ============================================================================
// RPC State
// ============================================================================

export interface RpcSessionState {
  model?: string;
  thinkingLevel: ThinkingLevel;
  isStreaming: boolean;
  isCompacting: boolean;
  steeringMode: "all" | "one-at-a-time";
  followUpMode: "all" | "one-at-a-time";
  sessionFile?: string;
  sessionId: string;
  session_name?: string;
  explicitSession: boolean;
  autoCompactionEnabled: boolean;
  queryCount: number;
  pendingMessageCount: number;
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
  interruptedRun?: InterruptedRunState | null;
  requestedRun?: RunTerminalState | null;
}

export interface ActiveRunState {
  runId: string;
  epoch: number;
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
