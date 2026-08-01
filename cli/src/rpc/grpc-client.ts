/**
 * Minimal gRPC client for FutureAgent CLI one-shot execution.
 * Uses @grpc/grpc-js with proto descriptor (same proto as TUI and agent).
 *
 * Unlike the TUI's GrpcClient, this client is designed for fire-and-forget
 * execution: connect → configure → prompt → stream output → disconnect.
 */

import * as grpc from "@grpc/grpc-js";
import * as protoLoader from "@grpc/proto-loader";
import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
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

const EMBEDDED_PROTO = `// future.proto — Protocol Buffers schema for FutureAgent
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

  // ── shell (execute shell command via the agent) ─────────────────────────

  // Shell command string.  Used when cmd_type = "shell".
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

  // List of tool names to enable (e.g. ["read", "write", "edit", "shell"]).
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

  // Optional run identity proposed by a client that has already created its
  // local run record. The Agent validates/adopts it atomically and returns the
  // canonical id in the prompt acknowledgement.
  string requested_run_id = 142;

  // Idempotency key for retrying StartRun independently of run identity.
  string client_request_id = 143;

  // Atomic behavior when the session already has an active run:
  // "reject_if_busy" (default), "enqueue_if_busy", or "supersede_session".
  // Empty is interpreted as "reject_if_busy" for backward compatibility.
  string busy_policy = 144;

  // ── set_sandbox_policy ─────────────────────────────────────────────────
  // Session sandbox + approval policy (typed sub-message, not JSON-in-string).
  // Read when type == "set_sandbox_policy".
  SandboxPolicy sandbox_policy = 150;

  // ── Attachments (GUI) ──────────────────────────────────────────────────
  // Structured attachments referenced by absolute local path. The agent
  // injects each file's path into the model-visible message (so the model can
  // read it with its own tools) and records the list in the user entry's meta.
  // Images additionally carry base64 and are sent as image_url when the active
  // model accepts image input; otherwise they degrade to a path reference.
  repeated Attachment attachments = 151;
}

// ── Attachment ───────────────────────────────────────────────────────────────
// A local file the user attached to a prompt. Files are NOT copied — the path
// is the original on-disk location, read on demand by the agent's tools.

message Attachment {
  // Absolute local filesystem path (original, not a workspace copy). For images
  // the agent reads + (down)encodes this to base64 itself — base64 never travels
  // over the wire.
  string path = 1;
  // "image" | "file".
  string kind = 2;
  // Display name (basename), for UI + the injected path block.
  string name = 3;

  reserved 4;  // was \`base64\` — images are now read from \`path\` on the agent

  // Optional absolute path to a cached thumbnail (images only). Not model-facing
  // — carried through to the user entry's meta so the GUI can render the chip
  // after a reload (messages are reconstructed from the agent JSONL).
  string thumbnail = 5;
}

// ── SandboxPolicy ────────────────────────────────────────────────────────────
// OS-sandbox boundary + approval policy for a session. The agent enforces the
// sandbox on spawned shell commands (Seatbelt on macOS) and uses the approval
// policy to decide when to raise approval requests. See gui/SANDBOX_PLAN.md.

message SandboxPolicy {
  // Rules live in on-disk files the agent reads directly (gui/APPROVAL_PLAN.md);
  // only the approval tier travels over the wire.
  reserved 1 to 5;  // v1: sandbox_mode / writable_roots / network_access / approval_policy / rules
  reserved 6;        // v2a: bool enabled (superseded by tier)
  // "off" (unrestricted) | "manual" (approval required) | "sandbox" (macOS Seatbelt, macOS only).
  string tier = 7;
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

  // Stable machine-readable error code. Additive: legacy handlers may leave it
  // empty and legacy clients may ignore it.
  string error_code = 7;

  // Optional JSON-serialised structured error details.
  string error_data = 8;
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

  // Atomic resume parameters. When atomic_attach is true, the server registers
  // the receiver and snapshots buffered events under the journal's same lock.
  string run_id = 3;
  int64 after_idx = 4;
  bool atomic_attach = 5;
}

// ── StreamEvent ─────────────────────────────────────────────────────────────

message StreamEvent {
  // Event type string (see StreamRequest.event_types).
  //
  // Canonical vocabulary (all clients key off these):
  //   agent_start / agent_end      run lifecycle (agent_start carries the run's
  //                                started_at_ms; agent_end carries error/usage/
  //                                duration_ms — the authoritative run totals)
  //   user_message                 a user turn (prompt / steer / follow-up)
  //   text_chunk                   assistant text token (the projected token stream)
  //   thinking_start / thinking_delta / thinking_end   reasoning stream
  //   tool_start                   tool execution began  {tool_id, tool_name, tool_args}
  //   tool_delta                   streaming tool-arg fragment {tool_id, text}
  //   tool_end                     tool execution finished {tool_id, text, error?}
  //   approval_request / approval_decision
  //   usage                        token accounting
  //   error                        run error
  //   tool_sandboxed / persistence_error / compaction_end   sideband signals
  //
  // Provider-specific aliases are normalized inside the Agent and never cross
  // this RPC boundary.
  string type = 1;

  // JSON-serialised event payload.  Structure depends on the event type.
  // Examples:
  //   text_chunk:    {"text": "Hello"}
  //   thinking_delta: {"text": "I need to..."}
  //   tool_start:    {"tool_id": "...", "tool_name": "read"}
  //   tool_end:      {"tool_id": "...", "text": "output..."}
  //   tool_delta:    {"tool_id": "...", "text": "partial args..."}
  //   approval_request: {"approval_request_id": "...", "tool_name": "shell", ...}
  //   agent_start:   {"started_at_ms": 1750000000000}
  //   agent_end:     {"error": "...", "usage": {"output_tokens": N}, "duration_ms": N}
  //                  (error present only on failure)
  string data = 2;

  // P1: client-side ordering/dedup. run_id is unique per user run (assigned once
  // at the is_streaming false→true edge); idx is monotonic within a run.
  string run_id = 3;
  int64 idx = 4;

  // When true, this frame replaces the consumer's local projection through
  // snapshot_cursor. It is returned by atomic AttachRun when the requested
  // cursor predates the bounded replay ring.
  bool projection_snapshot = 5;
  repeated ProjectedRunEvent snapshot_events = 6;
  int64 snapshot_cursor = 7;

  // Canonical run identity (P1 envelope). session_id scopes the event to its
  // conversation; epoch is the run's monotonic generation within the session
  // (a run accepted after an abort/restart gets a higher epoch). Together with
  // run_id + idx these let any consumer route, dedup and detect gaps without
  // external context.
  string session_id = 8;
  int64 epoch = 9;
}

// A compressed semantic event contained in a projection snapshot. Its idx is
// the latest source cursor folded into this event, preserving chronological
// ordering while allowing adjacent token deltas to be coalesced.
message ProjectedRunEvent {
  string type = 1;
  string data = 2;
  int64 idx = 3;
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
    timeoutSecs = 10,
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
        { deadline: grpcDeadline(timeoutSecs) },
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
              ? (response.data ? JSON.parse(response.data) : {})
              : response.data;
          const event: AgentEvent = {
            type: response.type || "message",
            sessionId: response.sessionId,
            runId: response.runId,
            epoch: Number(response.epoch ?? 0),
            idx: Number(response.idx ?? 0),
            projectionSnapshot: Boolean(response.projectionSnapshot),
            snapshotCursor: Number(response.snapshotCursor ?? 0),
            snapshotEvents: response.snapshotEvents ?? [],
            ...rawData,
          };
          events.push(event);

          if (response.type === "text_chunk") {
            const chunk = rawData?.text ?? "";
            text += chunk;
            if (onText) onText(chunk);
          } else if (response.type === "tool_start" && verbose) {
            const toolName = rawData?.tool_name || rawData?.name || "unknown";
            const toolInput = rawData?.tool_args || rawData?.input || "";
            const inputStr =
              typeof toolInput === "string" ? toolInput : JSON.stringify(toolInput);
            process.stderr.write(
              `\x1b[2m⚙ ${toolName}${inputStr ? " " + inputStr.slice(0, 80) : ""}\x1b[0m\n`,
            );
          } else if (response.type === "tool_end" && verbose) {
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

  // ─── Agent Commands ─────────────────────────────────────────────────

  async getAgentInfo(): Promise<{ version: string; skillsCount: number }> {
    return this.executeCommand("get_agent_info", {}, undefined, 5) as Promise<{
      version: string;
      skillsCount: number;
    }>;
  }

  // ─── Model Commands ──────────────────────────────────────────────────

  async listModels(): Promise<{
    models: Array<{
      id: string;
      label: string;
      provider: string;
      supportsImages: boolean;
      thinkingLevel: string;
      contextWindow: number;
      isDefault: boolean;
    }>;
    defaultModel: string;
  }> {
    return this.executeCommand("list_models", {}, undefined, 5) as Promise<{
      models: Array<{
        id: string;
        label: string;
        provider: string;
        supportsImages: boolean;
        thinkingLevel: string;
        contextWindow: number;
        isDefault: boolean;
      }>;
      defaultModel: string;
    }>;
  }

  // ─── Session Commands ────────────────────────────────────────────────

  async getState(sessionId?: string): Promise<RpcSessionState> {
    return this.executeCommand("get_state", {}, sessionId, 5) as Promise<RpcSessionState>;
  }

  async fork(entryId: string, sessionId?: string): Promise<{ cancelled: boolean; sessionId?: string }> {
    return this.executeCommand("fork", { entryId }, sessionId, 5) as Promise<{
      cancelled: boolean;
      sessionId?: string;
    }>;
  }

  async switchSession(sessionId: string): Promise<{ cancelled: boolean }> {
    return this.executeCommand("switch_session", { sessionId }, undefined, 5) as Promise<{
      cancelled: boolean;
    }>;
  }

  async listSessions(): Promise<{ sessions: SessionSummary[] }> {
    return this.executeCommand("list_sessions", {}, undefined, 5) as Promise<{
      sessions: SessionSummary[];
    }>;
  }

  async getSessionStats(sessionId: string): Promise<{
    sessionId: string;
    userMessages: number;
    assistantMessages: number;
    toolCalls: number;
    toolResults: number;
    totalMessages: number;
    tokens: { input: number; output: number; cacheRead: number; total: number };
    cost: number;
  }> {
    return this.executeCommand("get_session_stats", { sessionId }, undefined, 5) as Promise<{
      sessionId: string;
      userMessages: number;
      assistantMessages: number;
      toolCalls: number;
      toolResults: number;
      totalMessages: number;
      tokens: { input: number; output: number; cacheRead: number; total: number };
      cost: number;
    }>;
  }

  async renameSession(sessionId: string, name: string): Promise<void> {
    await this.executeCommand("set_session_name", { sessionId, name }, undefined, 5);
  }

  async deleteSession(sessionId: string): Promise<{ deleted: boolean }> {
    return this.executeCommand("delete_session", { sessionId }, undefined, 5) as Promise<{
      deleted: boolean;
    }>;
  }

  async getSessionEntries(sessionId: string): Promise<{
    entries: Array<Record<string, unknown>>;
  }> {
    return this.executeCommand("get_session_entries", { sessionId }, undefined, 5) as Promise<{
      entries: Array<Record<string, unknown>>;
    }>;
  }

  async newSession(cwd?: string): Promise<{ sessionId: string }> {
    return this.executeCommand("new_session", {
      cwd: cwd || process.cwd(),
      customInstructions: JSON.stringify({ createdBy: "cli" }),
    }, undefined, 5) as Promise<{ sessionId: string }>;
  }

  // ─── Config Commands ─────────────────────────────────────────────────

  async setModel(modelId: string, sessionId?: string): Promise<void> {
    await this.executeCommand("set_model", { modelId }, sessionId, 5);
  }

  async setThinkingLevel(level: ThinkingLevel, sessionId?: string): Promise<void> {
    await this.executeCommand("set_thinking_level", { level }, sessionId, 5);
  }

  async setTools(toolNames: string[], sessionId?: string): Promise<void> {
    await this.executeCommand("set_tools", { tools: toolNames }, sessionId, 5);
  }

  async disableTools(sessionId?: string): Promise<void> {
    await this.executeCommand("disable_tools", {}, sessionId, 5);
  }

  async disableBuiltinTools(sessionId?: string): Promise<void> {
    await this.executeCommand("disable_builtin_tools", {}, sessionId, 5);
  }

  async setSystemPrompt(prompt: string, sessionId?: string): Promise<void> {
    await this.executeCommand("set_system_prompt", { systemPrompt: prompt }, sessionId, 5);
  }

  async appendSystemPrompt(prompt: string, sessionId?: string): Promise<void> {
    await this.executeCommand("append_system_prompt", { systemPrompt: prompt }, sessionId, 5);
  }

  async setEphemeral(ephemeral: boolean, sessionId?: string): Promise<void> {
    await this.executeCommand("set_ephemeral", { ephemeral }, sessionId, 5);
  }

  async setPermissionLevel(level: PermissionLevel, sessionId?: string): Promise<void> {
    await this.executeCommand("set_permission_level", { level } as any, sessionId, 5);
  }

  async setCwd(cwd: string, sessionId?: string): Promise<void> {
    await this.executeCommand("set_cwd", { cwd }, sessionId, 5);
  }

  // ─── Prompt ─────────────────────────────────────────────────────────

  async prompt(message: string, sessionId?: string): Promise<void> {
    await this.executeCommand("prompt", { message }, sessionId, 30);
  }

  // ─── High-level Run ─────────────────────────────────────────────────

  /**
   * Execute a complete run: connect → configure → prompt → stream → return.
   * This is the main entry point for `future run`.
   */
  async run(config: RunConfig): Promise<RunResult> {
    const verbose = config.verbose ?? false;

    // 1. Establish session
    if (verbose) {
      process.stderr.write(`Connecting to ${this.address}...\n`);
    }

    let sessionId: string;

    // Resolve the target session: explicit links (fork, session, continue)
    // reuse an existing session; otherwise create a fresh one so --model and
    // other config changes are isolated to this run and never pollute the
    // default session.
    if (config.fork) {
      // Fork needs an explicit parent session — the agent no longer has a
      // default session to fall back to.  Without --session, fork from the
      // most recently updated session.
      let parentId = config.session;
      if (!parentId) {
        const { sessions } = await this.listSessions();
        if (sessions.length === 0) {
          throw new Error("No previous session to fork from.");
        }
        sessions.sort(
          (a, b) =>
            new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
        );
        parentId = sessions[0].id;
      }
      await this.switchSession(parentId);
      sessionId = parentId;
      if (verbose) {
        process.stderr.write(`Forking from entry ${config.fork}...\n`);
      }
      const result = await this.fork(config.fork, sessionId);
      if (result.cancelled) {
        throw new Error("Fork was cancelled");
      }
      if (result.sessionId) {
        sessionId = result.sessionId;
      }
    } else if (config.session) {
      await this.switchSession(config.session);
      sessionId = config.session;
      if (verbose) {
        process.stderr.write(`Switched to session ${config.session}\n`);
      }
    } else if (config.continueLast) {
      const { sessions } = await this.listSessions();
      if (sessions.length > 0) {
        sessions.sort(
          (a, b) =>
            new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
        );
        await this.switchSession(sessions[0].id);
        sessionId = sessions[0].id;
        if (verbose) {
          process.stderr.write(
            `Continuing session ${sessions[0].session_name || sessions[0].id}...\n`,
          );
        }
      } else {
        throw new Error("No previous session to continue; run without --continue to start a new one.");
      }
    } else {
      // Fresh session for every standalone run — isolates model/thinking/tool
      // changes so they never bleed into subsequent invocations.
      const newSession = await this.newSession(config.cwd);
      sessionId = newSession.sessionId;
      if (config.noSession) {
        await this.setEphemeral(true, sessionId);
        if (verbose) {
          process.stderr.write(`Created ephemeral session ${sessionId}\n`);
        }
      } else if (verbose) {
        process.stderr.write(`Created session ${sessionId}\n`);
      }
    }

    // 3. Apply configuration options (all scoped to this run's session)
    if (config.model) {
      if (verbose) process.stderr.write(`Model: ${config.model}\n`);
      await this.setModel(config.model, sessionId);
    }

    if (config.thinking) {
      if (verbose) process.stderr.write(`Thinking: ${config.thinking}\n`);
      await this.setThinkingLevel(config.thinking, sessionId);
    }

    if (config.tools && config.tools.length > 0) {
      await this.setTools(config.tools, sessionId);
    } else if (config.noTools) {
      await this.disableTools(sessionId);
    }

    if (config.noBuiltinTools) {
      await this.disableBuiltinTools(sessionId);
    }

    if (config.systemPrompt) {
      await this.setSystemPrompt(config.systemPrompt, sessionId);
    }

    if (config.appendSystemPrompt) {
      await this.appendSystemPrompt(config.appendSystemPrompt, sessionId);
    }

    if (config.permission) {
      if (verbose) process.stderr.write(`Permission: ${config.permission}\n`);
      await this.setPermissionLevel(config.permission, sessionId);
    }

    if (config.cwd) {
      await this.setCwd(config.cwd, sessionId);
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

    // 5. Send prompt (must target the same session as streamEvents)
    await this.prompt(config.message, sessionId);

    // 6. Wait for events to complete
    const { events, text } = await streamPromise;

    // 7. Get final state for model info (query the run's own session)
    let model: string | undefined;
    let thinkingLevel: string | undefined;
    try {
      const finalState = await this.getState(sessionId);
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

/**
 * Notify a running agent that skills were added or removed so it drops its
 * 60 s skills cache and re-discovers immediately.  Best-effort: if the
 * agent is not reachable (timeout 1 s) the call is silently dropped —
 * the next prompt triggers the TTL-based refresh anyway.
 *
 * The deadline is deliberately short: the agent is often not running when
 * skills are installed from a bare shell, and grpc-js retries connecting
 * until the deadline — a long timeout would stall every offline install.
 */
export async function notifyAgentRefreshSkills(grpcAddr?: string): Promise<void> {
  const address = grpcAddr ?? process.env.FUTURE_AGENT_GRPC_ADDR ?? "127.0.0.1:50051";
  const client = new proto.FutureAgent(address, grpc.credentials.createInsecure());
  try {
    const deadline = new Date();
    deadline.setSeconds(deadline.getSeconds() + 1); // 1 s timeout
    await new Promise<void>((resolve, reject) => {
      client.ExecuteCommand(
        { id: String(Date.now()), type: "refresh_skills" },
        deadline,
        (err: any) => (err ? reject(err) : resolve()),
      );
    });
  } catch {
    // Agent unreachable or not running — the cache TTL will pick it up.
  } finally {
    // Release the channel; without close() the client lingers until GC.
    client.close();
  }
}
