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
export const EMBEDDED_PROTO = `syntax = "proto3";

package proto;

option go_package = "github.com/futuregene/future-os/proto/go;proto";
option java_package = "ai.proto";
option java_multiple_files = true;

message RpcCommand {
  string id = 1;
  string type = 2;
  string message = 10;
  repeated ImageContent images = 11;
  string streaming_behavior = 12;
  string parent_session = 20;
  string model_id = 31;
  string level = 40;
  string mode = 50;
  string custom_instructions = 60;
  bool enabled = 70;
  string command = 80;
  string session_id = 91;
  string entry_id = 92;
  string name = 93;
  string cwd = 95;
  string system_prompt = 100;
  repeated string tools = 110;
  bool ephemeral = 120;
  repeated string enabled_models = 130;
}

message ImageContent {
  string type = 1;
  oneof content {
    string url = 10;
    string base64 = 11;
  }
}

message RpcResponse {
  string id = 1;
  string type = 2;
  string command = 3;
  bool success = 4;
  string data = 5;
  string error = 6;
}

message SessionState {
  string model = 1;
  string thinking_level = 2;
  bool is_streaming = 3;
  bool is_compacting = 4;
  string steering_mode = 5;
  string follow_up_mode = 6;
  string session_file = 7;
  string session_id = 8;
  string session_name = 9;
  bool explicit_session = 10;
  bool auto_compaction_enabled = 11;
  int32 query_count = 12;
  int32 pending_message_count = 13;
  string version = 14;
  string cwd = 15;
  repeated string skills = 16;
  repeated string context_files = 17;
  repeated string extensions = 18;
  int64 context_tokens = 19;
  int64 context_window = 20;
  double context_percent = 21;
  int64 tokens_in = 22;
  int64 tokens_out = 23;
  double total_cost = 24;
}

message SessionStats {
  string session_file = 1;
  string session_id = 2;
  int32 user_messages = 3;
  int32 assistant_messages = 4;
  int32 tool_calls = 5;
  int32 tool_results = 6;
  int32 total_messages = 7;
  TokenStats tokens = 8;
  double cost = 9;
}

message TokenStats {
  int32 input = 1;
  int32 output = 2;
  int32 cache_read = 3;
  int32 total = 4;
}

message Message {
  string role = 1;
  repeated ContentBlock content = 2;
  string name = 3;
  ToolCalls tool_calls = 4;
  ToolCall tool_call = 5;
  string text = 6;
}

message ContentBlock {
  string type = 1;
  string text = 10;
  string image_url = 11;
  string tool_use_id = 12;
  string tool_use_name = 13;
  string tool_use_input = 14;
  string tool_result_id = 15;
  string tool_result_content = 16;
}

message ToolCalls {
  repeated ToolCall calls = 1;
}

message ToolCall {
  string id = 1;
  string type = 2;
  FunctionCall function = 3;
}

message FunctionCall {
  string name = 1;
  string arguments = 2;
}

message BashResult {
  string output = 1;
  int32 exit_code = 2;
}

message CompactResult {
  int32 tokens_before = 1;
  int32 tokens_after = 2;
  string summary = 3;
  int32 messages_removed = 4;
}

message SessionListItem {
  string id = 1;
  string cwd = 2;
  string model = 3;
  int64 updated_at = 4;
}

service FutureAgent {
  rpc ExecuteCommand(RpcCommand) returns (RpcResponse);
  rpc StreamEvents(StreamRequest) returns (stream StreamEvent);
}

message StreamRequest {
  repeated string event_types = 1;
  string session_id = 2;
}

message StreamEvent {
  string type = 1;
  string data = 2;
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

  async getAvailableModels(): Promise<{ models: import("./types.js").ModelInfo[]; enabled_model_ids?: string[] }> {
    return this.call("get_available_models", {}) as Promise<{ models: import("./types.js").ModelInfo[]; enabled_model_ids?: string[] }>;
  }

  async setEnabledModels(modelIds: string[]): Promise<void> {
    await this.call("set_enabled_models", { enabledModels: modelIds });
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
