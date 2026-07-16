/**
 * Minimal gRPC client for FutureAgent CLI one-shot execution.
 * Uses @grpc/grpc-js with proto descriptor (same proto as TUI and agent).
 *
 * Unlike the TUI's GrpcClient, this client is designed for fire-and-forget
 * execution: connect → configure → prompt → stream output → disconnect.
 */

import * as grpc from "@grpc/grpc-js";
import * as protoLoader from "@grpc/proto-loader";
import * as fs from "node:fs";
import * as os from "node:os";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import type {
  RpcCommand,
  RpcSessionState,
  SessionSummary,
  AgentEvent,
  ThinkingLevel,
  PermissionLevel,
} from "./types.js";

// Inject Long globally for protobufjs (same as TUI's proto-setup.ts)
import Long from "long";
(globalThis as Record<string, unknown>).Long = Long;
(globalThis as Record<string, unknown>).dcodeIO = { Long };

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

// ─── Embedded Proto ──────────────────────────────────────────────────────

const EMBEDDED_PROTO = `syntax = "proto3";

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
  string provider = 30;
  string model_id = 31;
  string level = 40;
  string mode = 50;
  string custom_instructions = 60;
  bool enabled = 70;
  string command = 80;
  string session_path = 90;
  string session_id = 91;
  string entry_id = 92;
  string name = 93;
  string output_path = 94;
  string cwd = 95;
  string system_prompt = 100;
  repeated string tools = 110;
  bool no_tools = 111;
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
  int32 message_count = 12;
  int32 pending_message_count = 13;
  string version = 14;
  string cwd = 15;
  repeated string skills = 16;
  repeated string context_files = 17;
  repeated string extensions = 18;
  int32 context_tokens = 19;
  int32 context_window = 20;
  double context_percent = 21;
  int32 tokens_in = 22;
  int32 tokens_out = 23;
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

message ShellResult {
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

// ─── Proto Path Resolution ──────────────────────────────────────────────

function resolveProtoPath(): string {
  if (process.env.FUTURE_PROTO_PATH) {
    return process.env.FUTURE_PROTO_PATH;
  }
  const repoPath = join(__dirname, "..", "..", "..", "proto", "future.proto");
  if (fs.existsSync(repoPath)) {
    return repoPath;
  }
  const protoHash = createHash("sha256")
    .update(EMBEDDED_PROTO, "utf8")
    .digest("hex")
    .slice(0, 16);
  const tmpPath = join(os.tmpdir(), `future-proto-${protoHash}.proto`);
  if (!fs.existsSync(tmpPath)) {
    fs.writeFileSync(tmpPath, EMBEDDED_PROTO, "utf-8");
  }
  return tmpPath;
}

const PROTO_PATH = resolveProtoPath();

// ─── Proto Loading ──────────────────────────────────────────────────────

const packageDefinition = protoLoader.loadSync(PROTO_PATH, {
  keepCase: false,
  longs: String,
  enums: String,
  defaults: true,
  oneofs: true,
});

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const protoDescriptor = grpc.loadPackageDefinition(packageDefinition) as any;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const proto = protoDescriptor.proto as any;

// ─── Deadline Helper ────────────────────────────────────────────────────

function grpcDeadline(seconds = 30): Date {
  const d = new Date();
  d.setSeconds(d.getSeconds() + seconds);
  return d;
}

// ─── Run Configuration ──────────────────────────────────────────────────

export interface RunConfig {
  /** gRPC address (default: localhost:50051) */
  grpcAddr?: string;
  /** Fork from a session entry ID */
  fork?: string;
  /** Connect to a specific session */
  session?: string;
  /** Continue most recent session */
  continueLast?: boolean;
  /** Model ID (supports model:thinking format) */
  model?: string;
  /** Thinking level */
  thinking?: ThinkingLevel;
  /** Comma-separated tool names to enable */
  tools?: string[];
  /** Disable all tools */
  noTools?: boolean;
  /** Disable built-in tools only (keep extensions) */
  noBuiltinTools?: boolean;
  /** System prompt */
  systemPrompt?: string;
  /** Append to system prompt */
  appendSystemPrompt?: string;
  /** Working directory */
  cwd?: string;
  /** Permission level */
  permission?: PermissionLevel;
  /** Ephemeral mode (don't save session) */
  noSession?: boolean;
  /** Skill file/directory paths */
  skills?: string[];
  /** Disable skills */
  noSkills?: boolean;
  /** Output mode: text or json */
  mode?: "text" | "json";
  /** Show verbose progress to stderr */
  verbose?: boolean;
  /** The prompt message */
  message: string;
}

export interface RunResult {
  /** Session ID used */
  sessionId: string;
  /** Accumulated text output */
  text: string;
  /** All events (for JSON mode) */
  events: AgentEvent[];
  /** Model used */
  model?: string;
  /** Thinking level used */
  thinkingLevel?: string;
}

// ─── RunClient ──────────────────────────────────────────────────────────

export class RunClient {
  private client: any;
  private address: string;

  constructor(
    address = "localhost:50051",
    credentials?: grpc.ChannelCredentials,
  ) {
    this.address = address;
    const channelCredentials =
      credentials ?? grpc.credentials.createInsecure();
    this.client = new proto.FutureAgent(address, channelCredentials);
  }

  // ─── Low-level RPC ───────────────────────────────────────────────────

  private async executeCommand(
    type: string,
    cmd: Partial<RpcCommand>,
    sessionId?: string,
  ): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const request = {
        id: String(Date.now()),
        type,
        sessionId: sessionId || undefined,
        ...cmd,
      };

      this.client.ExecuteCommand(
        request,
        { deadline: grpcDeadline() },
        (err: Error | null, response: any) => {
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
        },
      );
    });
  }

  // ─── Stream Events ───────────────────────────────────────────────────

  /**
   * Stream events for a session. Returns a promise that resolves with all
   * accumulated events when the stream ends (agent_end received or timeout).
   */
  private streamEvents(
    sessionId: string,
    onText?: (text: string) => void,
    verbose?: boolean,
  ): Promise<{ events: AgentEvent[]; text: string }> {
    return new Promise((resolve, reject) => {
      const events: AgentEvent[] = [];
      let text = "";
      let done = false;
      const timeout = setTimeout(() => {
        if (!done) {
          done = true;
          stream.cancel();
          resolve({ events, text });
        }
      }, 300_000); // 5 min timeout

      const stream = this.client.StreamEvents({ sessionId });

      stream.on("data", (response: any) => {
        try {
          const rawData =
            typeof response.data === "string"
              ? JSON.parse(response.data)
              : response.data;
          const event: AgentEvent = {
            type: response.type || "message",
            ...rawData,
          };
          events.push(event);

          if (response.type === "text_chunk") {
            const chunk = rawData?.text ?? "";
            text += chunk;
            if (onText) onText(chunk);
          } else if (response.type === "tool_call" && verbose) {
            const toolName = rawData?.tool_name || rawData?.name || "unknown";
            const toolInput = rawData?.tool_input || rawData?.input || "";
            const inputStr =
              typeof toolInput === "string" ? toolInput : JSON.stringify(toolInput);
            process.stderr.write(
              `\x1b[2m⚙ ${toolName}${inputStr ? " " + inputStr.slice(0, 80) : ""}\x1b[0m\n`,
            );
          } else if (response.type === "tool_result" && verbose) {
            // Quiet — tool results can be large
          } else if (response.type === "agent_end") {
            done = true;
            clearTimeout(timeout);
            stream.cancel();
            resolve({ events, text });
          } else if (response.type === "error") {
            process.stderr.write(
              `\x1b[31mError: ${rawData?.error || "unknown"}\x1b[0m\n`,
            );
          }
        } catch {
          // Ignore parse errors
        }
      });

      stream.on("error", (err: Error) => {
        if (!done) {
          done = true;
          clearTimeout(timeout);
          reject(err);
        }
      });

      stream.on("end", () => {
        if (!done) {
          done = true;
          clearTimeout(timeout);
          resolve({ events, text });
        }
      });
    });
  }

  // ─── Session Commands ────────────────────────────────────────────────

  async getState(): Promise<RpcSessionState> {
    return this.executeCommand("get_state", {}) as Promise<RpcSessionState>;
  }

  async fork(entryId: string): Promise<{ cancelled: boolean; sessionId?: string }> {
    return this.executeCommand("fork", { entryId }) as Promise<{
      cancelled: boolean;
      sessionId?: string;
    }>;
  }

  async switchSession(sessionId: string): Promise<{ cancelled: boolean }> {
    return this.executeCommand("switch_session", { sessionId }) as Promise<{
      cancelled: boolean;
    }>;
  }

  async listSessions(): Promise<{ sessions: SessionSummary[] }> {
    return this.executeCommand("list_sessions", {}) as Promise<{
      sessions: SessionSummary[];
    }>;
  }

  async newSession(cwd?: string): Promise<{ sessionId: string }> {
    return this.executeCommand("new_session", {
      cwd: cwd || process.cwd(),
    }) as Promise<{ sessionId: string }>;
  }

  // ─── Config Commands ─────────────────────────────────────────────────

  async setModel(modelId: string): Promise<void> {
    await this.executeCommand("set_model", { modelId });
  }

  async setThinkingLevel(level: ThinkingLevel): Promise<void> {
    await this.executeCommand("set_thinking_level", { level });
  }

  async setTools(toolNames: string[]): Promise<void> {
    await this.executeCommand("set_tools", { tools: toolNames });
  }

  async disableTools(): Promise<void> {
    await this.executeCommand("disable_tools", {});
  }

  async disableBuiltinTools(): Promise<void> {
    await this.executeCommand("disable_builtin_tools", {});
  }

  async setSystemPrompt(prompt: string): Promise<void> {
    await this.executeCommand("set_system_prompt", { systemPrompt: prompt });
  }

  async appendSystemPrompt(prompt: string): Promise<void> {
    await this.executeCommand("append_system_prompt", { systemPrompt: prompt });
  }

  async setEphemeral(ephemeral: boolean): Promise<void> {
    await this.executeCommand("set_ephemeral", { ephemeral });
  }

  async setPermissionLevel(level: PermissionLevel): Promise<void> {
    await this.executeCommand("set_permission_level", { level } as any);
  }

  async setCwd(cwd: string): Promise<void> {
    await this.executeCommand("set_cwd", { cwd });
  }

  // ─── Prompt ─────────────────────────────────────────────────────────

  async prompt(message: string): Promise<void> {
    await this.executeCommand("prompt", { message });
  }

  // ─── High-level Run ─────────────────────────────────────────────────

  /**
   * Execute a complete run: connect → configure → prompt → stream → return.
   * This is the main entry point for `future run`.
   */
  async run(config: RunConfig): Promise<RunResult> {
    const verbose = config.verbose ?? false;

    // 1. Get initial state (also establishes session)
    if (verbose) {
      process.stderr.write(`Connecting to ${this.address}...\n`);
    }
    const state = await this.getState();
    let sessionId = state.sessionId;

    // 2. Handle session options (fork / continue / switch)
    if (config.fork) {
      if (verbose) {
        process.stderr.write(`Forking from entry ${config.fork}...\n`);
      }
      const result = await this.fork(config.fork);
      if (result.cancelled) {
        throw new Error("Fork was cancelled");
      }
      if (result.sessionId) {
        sessionId = result.sessionId;
      }
    } else if (config.session) {
      if (verbose) {
        process.stderr.write(`Switching to session ${config.session}...\n`);
      }
      await this.switchSession(config.session);
      sessionId = config.session;
    } else if (config.continueLast) {
      const { sessions } = await this.listSessions();
      if (sessions.length > 0) {
        sessions.sort(
          (a, b) =>
            new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
        );
        if (verbose) {
          process.stderr.write(
            `Continuing session ${sessions[0].name || sessions[0].id}...\n`,
          );
        }
        await this.switchSession(sessions[0].id);
        sessionId = sessions[0].id;
      }
    }

    // 3. Apply configuration options
    if (config.model) {
      if (verbose) process.stderr.write(`Model: ${config.model}\n`);
      await this.setModel(config.model);
    }

    if (config.thinking) {
      if (verbose) process.stderr.write(`Thinking: ${config.thinking}\n`);
      await this.setThinkingLevel(config.thinking);
    }

    if (config.tools && config.tools.length > 0) {
      await this.setTools(config.tools);
    } else if (config.noTools) {
      await this.disableTools();
    }

    if (config.noBuiltinTools) {
      await this.disableBuiltinTools();
    }

    if (config.systemPrompt) {
      await this.setSystemPrompt(config.systemPrompt);
    }

    if (config.appendSystemPrompt) {
      await this.appendSystemPrompt(config.appendSystemPrompt);
    }

    if (config.permission) {
      if (verbose) process.stderr.write(`Permission: ${config.permission}\n`);
      await this.setPermissionLevel(config.permission);
    }

    if (config.noSession) {
      await this.setEphemeral(true);
    }

    if (config.cwd) {
      await this.setCwd(config.cwd);
    }

    // 4. Start streaming events BEFORE sending prompt
    if (verbose) process.stderr.write("Running...\n");
    const streamPromise = this.streamEvents(
      sessionId,
      config.mode !== "json"
        ? (chunk) => process.stdout.write(chunk)
        : undefined,
      verbose,
    );

    // 5. Send prompt
    await this.prompt(config.message);

    // 6. Wait for events to complete
    const { events, text } = await streamPromise;

    // 7. Get final state for model info
    let model: string | undefined;
    let thinkingLevel: string | undefined;
    try {
      const finalState = await this.getState();
      model = finalState.model;
      thinkingLevel = finalState.thinkingLevel;
    } catch {
      // Ignore — state query after completion is non-critical
    }

    // 8. Output (for text mode, already streamed to stdout)
    if (config.mode === "json") {
      const result = {
        sessionId,
        model,
        thinkingLevel,
        text,
        messages: events,
      };
      process.stdout.write(JSON.stringify(result, null, 2) + "\n");
    } else {
      // Add trailing newline if text doesn't already end with one
      if (text && !text.endsWith("\n")) {
        process.stdout.write("\n");
      }
    }

    return { sessionId, text, events, model, thinkingLevel };
  }
}
