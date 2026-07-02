/**
 * gRPC client for FutureAgent.
 * Uses @grpc/grpc-js with proto descriptor.
 * Only supports gRPC (no JSON-RPC or Unix socket).
 */

// Must import proto-setup BEFORE any gRPC modules — it injects Long globally
// for protobufjs, which does a dynamic global lookup in bun build --compile.
import "./proto-setup.js";

import * as grpc from "@grpc/grpc-js";
import * as protoLoader from "@grpc/proto-loader";
import * as fs from "node:fs";
import * as os from "node:os";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import type {
  RpcCommand,
  RpcSessionState,
  SessionSummary,
  AgentEvent,
} from "./types.js";

export type EventListener = (event: AgentEvent) => void;

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// Embedded proto content for standalone binaries (no external file dependency).
// Generated from ../../proto/future.proto at build time.
export const EMBEDDED_PROTO = `// future.proto — Protocol Buffers schema for FutureAgent
//
// This is the canonical API definition for the FutureAgent engine.
// Generated Rust code (agent/src/grpc/generated/proto.rs) is used by
// the agent, channel bridge, TUI, and CLI.
//
// Field numbers are stable and MUST NOT be reused.

syntax = "proto3";

package proto;

option go_package = "github.com/futuregene/future-os/proto/go;proto";
option java_package = "ai.proto";
option java_multiple_files = true;

// =============================================================================
// RPC Commands — sent by clients (TUI / channel bridge / CLI) to the agent
// =============================================================================

message RpcCommand {
  // Unique request correlation ID (UUID v4).  Echoed back in RpcResponse.id
  // so the client can match requests to responses.
  string id = 1;

  // Command name, e.g. "prompt", "get_state", "new_session", "abort".
  // Determines which handler processes this request.
  string type = 2;

  // ── Prompting ──────────────────────────────────────────────────────────

  // User prompt text.  Required for "prompt", "steer", "follow_up".
  string message = 10;

  // Images attached to the prompt (base64, URL, or file path).
  repeated ImageContent images = 11;

  // How to queue the prompt: "steer" (interrupt current run) or
  // "followUp" (enqueue after current run completes).
  string streaming_behavior = 12;

  // ── fork / new_session ─────────────────────────────────────────────────

  // Parent session ID when forking.  If empty, fork uses the current
  // session.  Also used by new_session to record lineage.
  string parent_session = 20;

  // ── set_model ──────────────────────────────────────────────────────────

  // Canonical model ID.  If it contains a "/", the part before the slash
  // is treated as the provider.  Example: "deepseek/deepseek-chat".
  string model_id = 31;

  // ── set_thinking_level ─────────────────────────────────────────────────

  // Thinking level: "off", "minimal", "low", "medium", "high", "xhigh".
  string level = 40;

  // ── set_steering_mode / set_follow_up_mode ─────────────────────────────

  // Queue mode: "all" (accept all) or "one-at-a-time" (replace pending).
  string mode = 50;

  // ── compact ────────────────────────────────────────────────────────────

  // Optional custom instructions for the compaction summariser.
  string custom_instructions = 60;

  // ── set_auto_compaction / set_auto_retry ───────────────────────────────

  // Toggle flag (true = on, false = off).
  bool enabled = 70;

  // ── bash (execute shell command via the agent) ─────────────────────────

  // Shell command string.  Used when cmd_type = "bash".
  string command = 80;

  // ── Session bookkeeping ────────────────────────────────────────────────

  // Target session ID.  Almost every command requires this so the
  // agent knows which session to operate on.  new_session uses it
  // as the requested ID (generated if empty).
  string session_id = 91;

  // Entry ID within a session (e.g. a specific tool-call for approval).
  string entry_id = 92;

  // Session name (set by /name command).  Used with set_session_name.
  string name = 93;

  // ── new_session cwd ────────────────────────────────────────────────────

  // Working directory for the new session.  The agent resolves "~" and
  // relative paths.  Defaults to ~/.future/agent/workspace.
  string cwd = 95;

  // ── set_system_prompt ──────────────────────────────────────────────────

  // Custom system prompt that replaces or appends to the built-in prompt.
  string system_prompt = 100;

  // ── set_tools / disable_tools ──────────────────────────────────────────

  // List of tool names to enable (e.g. ["read", "write", "edit", "bash"]).
  repeated string tools = 110;

  // ── set_ephemeral ──────────────────────────────────────────────────────

  // If true, the session is not persisted to disk.
  bool ephemeral = 120;

  // ── set_enabled_models ─────────────────────────────────────────────────

  // List of model IDs that the user is allowed to select.  Empty means
  // all models are available.
  repeated string enabled_models = 130;

  // ── get_events_since (P1) ──────────────────────────────────────────────
  // Replay current-run events with idx > since_idx; run_id scopes the request
  // (a mismatch means the run rolled over and the caller must realign).
  int64 since_idx = 140;
  string run_id = 141;
}

// ── ImageContent ───────────────────────────────────────────────────────────

message ImageContent {
  // Image source type: "image_url", "image_base64", or "image_file".
  string type = 1;

  // Mutually exclusive content reference.
  oneof content {
    // Remote image URL (HTTP/HTTPS).
    string url = 10;
    // Base64-encoded image data.
    string base64 = 11;
  }

  // Local filesystem path after the image is saved to disk.
  string file_path = 12;
}

// =============================================================================
// RPC Responses — returned by the agent for every ExecuteCommand call
// =============================================================================

message RpcResponse {
  // Echo of the request ID for correlation.
  string id = 1;

  // Fixed literal "response".
  string type = 2;

  // The command this response belongs to (echo of RpcCommand.type).
  string command = 3;

  // true on success, false on error.
  bool success = 4;

  // JSON-serialised response payload.  Structure depends on the command.
  string data = 5;

  // Error message when success is false.
  string error = 6;
}

// =============================================================================
// Session State — returned by get_state (the fields displayed in /status)
// =============================================================================

message SessionState {
  // Currently active model ID (e.g. "deepseek-v4-pro").
  string model = 1;

  // Thinking / effort level: "off", "minimal", "low", "medium", "high", "xhigh".
  string thinking_level = 2;

  // Whether the agent loop is currently processing a prompt.
  bool is_streaming = 3;

  // Whether a compaction run is in progress (always false in current code).
  bool is_compacting = 4;

  // Steering queue mode: "all" or "one-at-a-time".
  string steering_mode = 5;

  // Follow-up queue mode: "all" or "one-at-a-time".
  string follow_up_mode = 6;

  // Reserved for session file path.  Always null in current code.
  string session_file = 7;

  // Current session ID (unique, generated on creation).
  string session_id = 8;

  // User-assigned session name, or empty if unnamed.
  string session_name = 9;

  // Whether this session was explicitly created via /new (vs. auto-created).
  bool explicit_session = 10;

  // Whether automatic context compaction is enabled.
  bool auto_compaction_enabled = 11;

  // Number of user messages (prompts + steer + follow_up).  Excludes
  // internal tool/assistant messages.  Displayed as "Queries" in /status.
  int32 query_count = 12;

  // Number of messages queued but not yet processed (steering + follow_up).
  int32 pending_message_count = 13;

  // Agent version string (from Cargo.toml).
  string version = 14;

  // Working directory for the session.
  string cwd = 15;

  // Discovered skill names available in this session.
  repeated string skills = 16;

  // Context file paths loaded via CLAUDE.md / AGENTS.md / GEMINI.md.
  repeated string context_files = 17;

  // Reserved for UI extensions.  Always null in current code.
  repeated string extensions = 18;

  // Current estimated context token count (from last API call's prompt_tokens,
  // with fallback to heuristic estimation).
  int64 context_tokens = 19;

  // Model's maximum context window in tokens.
  int64 context_window = 20;

  // context_tokens as a percentage of context_window (0.0–100.0).
  double context_percent = 21;

  // Cumulative input tokens consumed in this session.
  int64 tokens_in = 22;

  // Cumulative output tokens produced in this session.
  int64 tokens_out = 23;

  // Cumulative cost in CNY (¥).
  double total_cost = 24;

  // Whether the current model supports image input (multimodal).
  bool image_support = 25;

  // Cumulative cache-read tokens (prompt caching hits).
  int64 tokens_cache_r = 26;

  // Cumulative cache-write tokens (prompt caching writes).
  int64 tokens_cache_w = 27;

  // Tool execution permission level: "all" (unrestricted), "workspace"
  // (cwd only), or "none" (read-only tools).
  string permission_level = 28;
}

// =============================================================================
// gRPC Service Definition
// =============================================================================

service FutureAgent {
  // Unary RPC: send a command, get a response.
  // Used by the TUI and channel bridge for all non-streaming operations
  // (prompt, get_state, new_session, abort, set_model, etc.).
  rpc ExecuteCommand(RpcCommand) returns (RpcResponse);

  // Server-side streaming RPC: subscribe to agent events.
  // The TUI uses this for real-time text/tool/thinking updates.
  rpc StreamEvents(StreamRequest) returns (stream StreamEvent);
}

// ── StreamRequest ───────────────────────────────────────────────────────────

message StreamRequest {
  // Optional list of event types to receive.  Empty = all events.
  // Valid types: "ping", "agent_start", "agent_end", "text_chunk",
  // "thinking_start", "thinking_delta", "thinking_end", "tool_start",
  // "tool_delta", "tool_end", "approval_request", "error", "stop".
  repeated string event_types = 1;

  // Scope events to a specific session.  Required so the agent
  // knows which session's broadcaster to subscribe to.
  string session_id = 2;
}

// ── StreamEvent ─────────────────────────────────────────────────────────────

message StreamEvent {
  // Event type string (see StreamRequest.event_types).
  string type = 1;

  // JSON-serialised event payload.  Structure depends on the event type.
  // Examples:
  //   text_chunk:    {"text": "Hello"}
  //   thinking_delta: {"text": "I need to..."}
  //   tool_start:    {"tool_id": "...", "tool_name": "read"}
  //   tool_end:      {"tool_id": "...", "text": "output..."}
  //   tool_delta:    {"tool_id": "...", "text": "partial args..."}
  //   approval_request: {"approval_request_id": "...", "tool_name": "bash", ...}
  //   agent_end:     {"error": "..."}  (error present only on failure)
  string data = 2;

  // P1: client-side ordering/dedup. run_id is unique per user run (assigned once
  // at the is_streaming false→true edge); idx is monotonic within a run.
  string run_id = 3;
  int64 idx = 4;
}
`;

// Resolve proto path: env var > repo-relative > temp file from embedded content
function resolveProtoPath(): string {
  // 1. Env override
  if (process.env.FUTURE_PROTO_PATH) {
    return process.env.FUTURE_PROTO_PATH;
  }
  // 2. Repo-relative path (development)
  const repoPath = join(__dirname, "..", "..", "..", "proto", "future.proto");
  if (fs.existsSync(repoPath)) {
    return repoPath;
  }
  // 3. Standalone binary: write embedded proto to temp file
  const tmpPath = join(os.tmpdir(), "future-proto-v0.3.0.proto");
  if (!fs.existsSync(tmpPath)) {
    fs.writeFileSync(tmpPath, EMBEDDED_PROTO, "utf-8");
  }
  return tmpPath;
}

const PROTO_PATH = resolveProtoPath();

// ─── Proto Setup ─────────────────────────────────────────────────────────

const packageDefinition = protoLoader.loadSync(PROTO_PATH, {
  keepCase: false,
  longs: String,
  enums: String,
  defaults: true,
  oneofs: true,
});

const protoDescriptor = grpc.loadPackageDefinition(packageDefinition) as any;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const proto = protoDescriptor.proto as any;

// ─── RPC Client ─────────────────────────────────────────────────────────

export class GrpcClient {
  private client: any;
  private eventListeners: EventListener[] = [];
  private streamCall: any = null;
  private connected = false;
  private currentSessionId: string = "";

  constructor(address = "localhost:50051") {
    const credentials = grpc.credentials.createInsecure();
    this.client = new proto.FutureAgent(address, credentials);
  }

  // ─── Session Management ───────────────────────────────────────────────

  getCurrentSessionId(): string {
    return this.currentSessionId;
  }

  setCurrentSessionId(sessionId: string): void {
    this.currentSessionId = sessionId;
  }

  // ─── Event Streaming ─────────────────────────────────────────────────

  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  connectEvents(): void {
    // Cancel existing stream and timer
    if (this.streamCall) {
      this.streamCall.cancel();
      this.streamCall = null;
    }
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }

    const scheduleReconnect = () => {
      if (!this.reconnectTimer) {
        this.connected = false;
        this.reconnectTimer = setTimeout(() => {
          this.reconnectTimer = null;
          this.connectEvents();
        }, 2000);
      }
    };

    let call;
    try {
      call = this.client.StreamEvents({
        sessionId: this.currentSessionId,
      });
    } catch (_err) {
      // StreamEvents() threw synchronously (channel dead)
      scheduleReconnect();
      return;
    }
    this.streamCall = call;

    call.on("data", (response: any) => {
      if (!this.connected) {
        this.connected = true;
      }
      try {
        const rawData = typeof response.data === "string" ? JSON.parse(response.data) : response.data;
        const { type: _dataType, ...rest } = rawData || {};
        const event: AgentEvent = {
          type: response.type || "message",
          ...rest,
        };

        for (const listener of this.eventListeners) {
          try {
            listener(event);
          } catch {
            // Ignore listener errors
          }
        }
      } catch {
        // Ignore parse errors
      }
    });

    call.on("end", () => {
      if (this.streamCall === call) {
        this.streamCall = null;
        scheduleReconnect();
      }
    });

    call.on("error", (_err: Error) => {
      if (this.streamCall === call) {
        this.streamCall = null;
        scheduleReconnect();
      }
    });
    // Note: connected is set to true only when first stream data arrives
    // (see "data" handler above), not here. The StreamEvents call creates
    // the stream but the gRPC channel may not be ready for unary RPCs yet.
  }


  isConnected(): boolean {
    return this.connected;
  }

  subscribe(listener: EventListener): () => void {
    this.connectEvents();
    this.eventListeners.push(listener);
    return () => {
      this.eventListeners = this.eventListeners.filter((l) => l !== listener);
    };
  }

  disconnect(): void {
    this.streamCall?.cancel();
    this.streamCall = null;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.connected = false;
  }

  // ─── RPC Call Helper ─────────────────────────────────────────────────

  private async call(type: string, cmd: Partial<RpcCommand>, retry = true): Promise<unknown> {
    // Wait for connection if not yet connected (first call or reconnecting)
    if (!this.connected && !this.reconnectTimer) {
      this.connectEvents();
    }
    // Brief wait for connection to establish
    const start = Date.now();
    while (!this.connected && Date.now() - start < 5000) {
      await new Promise(r => setTimeout(r, 100));
    }

    const doCall = (): Promise<unknown> => new Promise((resolve, reject) => {
      const request = {
        id: String(Date.now()),
        type,
        sessionId: this.currentSessionId || undefined,
        ...cmd,
      };

      const deadline = new Date();
      deadline.setSeconds(deadline.getSeconds() + 30);
      this.client.ExecuteCommand(request, { deadline }, (err: Error | null, response: any) => {
        if (err) {
          reject(err);
          return;
        }
        if (!response.success) {
          reject(new Error(response.error || "unknown error"));
          return;
        }
        if (response.data && typeof response.data === "string") {
          try {
            resolve(JSON.parse(response.data));
          } catch {
            resolve(response.data);
          }
        } else {
          resolve(response.data);
        }
      });
    });

    try {
      return await doCall();
    } catch (err: any) {
      // On transport error, trigger reconnect so stream comes back.
      // Don't retry the call itself — for non-idempotent commands like
      // 'prompt', the request may have already reached the agent and
      // we'd create a duplicate. The stream will deliver events either way.
      const msg = err?.message || String(err);
      const isTransport = msg.includes("transport") || msg.includes("14 UNAVAILABLE")
        || msg.includes("Connect Failed") || msg.includes("ECONNREFUSED");
      if (isTransport) {
        this.connected = false;
        this.connectEvents();
      }
      throw err;
    }
  }

  // ─── Session Management RPC Methods ──────────────────────────────────

  async newSession(): Promise<{ sessionId?: string; cancelled: boolean }> {
    const result = await this.call("new_session", { cwd: process.cwd() }) as any;
    if (result?.sessionId) {
      this.currentSessionId = result.sessionId;
      this.connectEvents();
    }
    return result || { cancelled: false };
  }

  async switchSession(sessionId: string): Promise<{ cancelled: boolean }> {
    const result = await this.call("switch_session", { sessionId }) as any;
    if (result && !result.cancelled) {
      this.currentSessionId = sessionId;
      this.connectEvents();
    }
    return result || { cancelled: false };
  }

  async fork(entryId: string): Promise<{ text: string; cancelled: boolean }> {
    const result = await this.call("fork", { entryId }) as any;
    if (result?.sessionId) {
      this.currentSessionId = result.sessionId;
      this.connectEvents();
    }
    return result || { text: "", cancelled: true };
  }

  async clone(): Promise<{ cancelled: boolean }> {
    const result = await this.call("clone", {}) as any;
    if (result?.sessionId) {
      this.currentSessionId = result.sessionId;
      this.connectEvents();
    }
    return result || { cancelled: true };
  }

  async getForkMessages(): Promise<{ messages: unknown[] }> {
    return this.call("get_fork_messages", {}) as Promise<{ messages: unknown[] }>;
  }

  async getLastAssistantText(): Promise<{ text: string | null }> {
    return this.call("get_last_assistant_text", {}) as Promise<{ text: string | null }>;
  }

  async setSessionName(name: string): Promise<void> {
    await this.call("set_session_name", { name });
  }

  async listSessions(): Promise<{ sessions: SessionSummary[] }> {
    return this.call("list_sessions", {}) as Promise<{ sessions: SessionSummary[] }>;
  }

  async deleteSession(sessionId: string): Promise<{ deleted: boolean }> {
    return this.call("delete_session", { sessionId }) as Promise<{ deleted: boolean }>;
  }

  // ─── Core RPC Methods ────────────────────────────────────────────────

  async prompt(message: string, images?: RpcCommand["images"], streamingBehavior?: "steer" | "followUp"): Promise<void> {
    await this.call("prompt", { message, images, streamingBehavior });
  }

  async steer(message: string): Promise<void> {
    await this.call("steer", { message });
  }

  async followUp(message: string): Promise<void> {
    await this.call("follow_up", { message });
  }

  async abort(): Promise<void> {
    await this.call("abort", {});
  }

  async getState(): Promise<RpcSessionState> {
    return this.call("get_state", {}) as Promise<RpcSessionState>;
  }

  async getMessages(): Promise<{ messages: unknown[] }> {
    return this.call("get_messages", {}) as Promise<{ messages: unknown[] }>;
  }

  async setModel(modelId: string): Promise<void> {
    await this.call("set_model", { modelId });
  }

  async cycleModel(): Promise<{ model: string; thinkingLevel: string; isScoped: boolean } | null> {
    return this.call("cycle_model", {}) as Promise<{ model: string; thinkingLevel: string; isScoped: boolean } | null>;
  }

  async listModels(): Promise<import("./types.js").ModelInfo[]> {
    const resp = await this.call("list_models", {}) as { models: import("./types.js").ModelInfo[] };
    return resp.models;
  }

  async setEnabledModels(modelIds: string[]): Promise<void> {
    // Stored client-side in TUI settings; no longer persisted on agent.
  }

  async setThinkingLevel(level: RpcCommand["level"]): Promise<void> {
    await this.call("set_thinking_level", { level });
  }

  async cycleThinkingLevel(): Promise<{ level: string } | null> {
    return this.call("cycle_thinking_level", {}) as Promise<{ level: string } | null>;
  }

  async setSteeringMode(mode: "all" | "one-at-a-time"): Promise<void> {
    await this.call("set_steering_mode", { mode });
  }

  async setFollowUpMode(mode: "all" | "one-at-a-time"): Promise<void> {
    await this.call("set_follow_up_mode", { mode });
  }

  async compact(customInstructions?: string): Promise<string> {
    return this.call("compact", { customInstructions }) as Promise<string>;
  }

  async setAutoCompaction(enabled: boolean): Promise<void> {
    await this.call("set_auto_compaction", { enabled });
  }

  async setAutoRetry(enabled: boolean): Promise<void> {
    await this.call("set_auto_retry", { enabled });
  }

  async setCwd(cwd: string): Promise<void> {
    await this.call("set_cwd", { cwd });
  }

  async approvalDecision(requestId: string, approved: boolean, note?: string): Promise<void> {
    await this.call("approval_decision", {
      mode: approved ? "approved" : "rejected",
      message: note || "",
      entryId: requestId,
    } as any);
  }

  async setPermissionLevel(level: "all" | "workspace" | "none"): Promise<void> {
    await this.call("set_permission_level", { level } as any);
  }

  async abortRetry(): Promise<void> {
    await this.call("abort_retry", {});
  }

  async bash(command: string): Promise<unknown> {
    return this.call("bash", { command });
  }

  async abortBash(): Promise<void> {
    await this.call("abort_bash", {});
  }

  async getSessionStats(): Promise<unknown> {
    return this.call("get_session_stats", {});
  }

  async exportHtml(outputPath?: string): Promise<{ path: string }> {
    return this.call("export_html", { outputPath }) as Promise<{ path: string }>;
  }

  async getCommands(): Promise<{ commands: unknown[] }> {
    return this.call("get_commands", {}) as Promise<{ commands: unknown[] }>;
  }

  async reloadConfig(): Promise<{ skills: string[]; contextFiles: string[] }> {
    return this.call("reload_config", {}) as Promise<{ skills: string[]; contextFiles: string[] }>;
  }
}
