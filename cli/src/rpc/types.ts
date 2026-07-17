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
  // set_system_prompt / append_system_prompt
  systemPrompt?: string;
  // set_tools / disable_tools / disable_builtin_tools
  tools?: string[];
  noTools?: boolean;
  // set_ephemeral
  ephemeral?: boolean;
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
}

// ============================================================================
// RPC State (from get_state)
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
  sessionName?: string;
  explicitSession: boolean;
  autoCompactionEnabled: boolean;
  messageCount: number;
  pendingMessageCount: number;
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
}

// ============================================================================
// Session Summary (from list_sessions)
// ============================================================================

export interface SessionSummary {
  id: string;
  cwd: string;
  updated_at: string;
  model: string;
  name?: string;
  parent_session_id?: string;
  first_message?: string;
  query_count?: number;
}

// ============================================================================
// Model Info (from get_available_models)
// ============================================================================

export interface ModelInfo {
  id: string;
  name: string;
  provider: string;
  reasoning: boolean;
  image: boolean;
  contextWindow: number;
  maxTokens: number;
}

// ============================================================================
// Agent Events (from StreamEvents)
// ============================================================================

export interface AgentEvent {
  type: string;
  text?: string;
  tool_id?: string;
  tool_name?: string;
  [key: string]: unknown;
}
