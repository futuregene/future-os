/**
 * RPC types for FutureAgent communication.
 * Mirrors the TUI rpc/types.ts and the Rust rpc module on the server side.
 * Used by the CLI `run` command for one-shot agent execution.
 */

// ============================================================================
// RPC Command (matches proto RpcCommand)
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
  // set_system_prompt / append_system_prompt
  systemPrompt?: string;
  // set_tools / disable_tools / disable_builtin_tools
  tools?: string[];
  noTools?: boolean;
  // set_ephemeral
  ephemeral?: boolean;
  runId?: string;
  sinceIdx?: number;
  requestedRunId?: string;
  clientRequestId?: string;
  busyPolicy?: "reject_if_busy" | "enqueue_if_busy" | "supersede_session";
}

// ============================================================================
// Types
// ============================================================================

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
  error?: string;
  errorCode?: string;
  errorData?: unknown;
}

// ============================================================================
// RPC State (from get_state)
// ============================================================================

export interface RpcSessionState {
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
  version?: string;
  cwd?: string;
  permissionLevel?: PermissionLevel;
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
  first_message?: string;
  query_count?: number;
}

// ============================================================================
// Model Info (from get_available_models)
// ============================================================================

export interface ModelInfo {
  id: string;
  label: string;            // display name
  provider: string;
  supportsImages: boolean;
  thinkingLevel: string;    // default thinking level for this model ("off"/"high"/...)
  contextWindow: number;
  isDefault: boolean;
}

// ============================================================================
// Agent Events (from StreamEvents)
// ============================================================================

export interface AgentEvent {
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
  text?: string;
  tool_id?: string;
  tool_name?: string;
  [key: string]: unknown;
}

export interface ProjectedRunEvent {
  type: string;
  data: string;
  idx: number;
}
