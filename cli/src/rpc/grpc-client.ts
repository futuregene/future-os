/**
 * Minimal gRPC client for FutureAgent CLI one-shot execution.
 * Uses @grpc/grpc-js with proto descriptor (same proto as TUI and agent).
 *
 * Unlike the TUI's GrpcClient, this client is designed for fire-and-forget
 * execution: connect → configure → prompt → stream output → disconnect.
 */

import * as grpc from "@grpc/grpc-js";
import { loadGrpcAgentProto, responseData, streamEventData } from "@future-os/rpc";
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

// ─── Proto Loading ──────────────────────────────────────────────────────

// The proto schema and its grpc-js loader live in the shared wire-contract
// package (@future-os/rpc) — single source of truth for every TS client.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const proto: any = loadGrpcAgentProto();

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
          // Shared wire decode: typed payload when present, JSON `data` fallback.
          resolve(responseData(response));
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
          // Shared wire decode: typed payload when present, JSON `data`
          // fallback; the injected `type` key is dropped either way.
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          const rawData = streamEventData(response) as Record<string, any>;
          const event: AgentEvent = {
            type: response.type || "message",
            sessionId: response.sessionId,
            runId: response.runId,
            epoch: Number(response.epoch ?? 0),
            idx: Number(response.idx ?? 0),
            eventId: response.eventId,
            timestamp: response.timestamp,
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
      createdBy: "cli",
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
