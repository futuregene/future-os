/**
 * JSON-RPC types for xihu_tui Agent communication.
 * Mirrors internal/rpc/types.go on the Go server side.
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
  // bash
  command?: string;
  // Session
  sessionPath?: string;
  sessionId?: string;
  entryId?: string;
  name?: string;
  outputPath?: string;
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
  sessionName?: string;
  explicitSession: boolean;
  autoCompactionEnabled: boolean;
  messageCount: number;
  pendingMessageCount: number;
  // Welcome info
  version?: string;
  cwd?: string;
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
}

// ============================================================================
// Agent Events
// ============================================================================

export type AgentEvent = {
  type: string;
  text?: string;   // text_chunk, agent_end, tool_delta
  tool_id?: string;
  tool_name?: string;
  [key: string]: unknown;
};
