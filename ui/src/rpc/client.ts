/**
 * JSON-RPC client for xihu Agent.
 */

import http from "node:http";
import https from "node:https";
import { URL } from "node:url";
import type {
  RpcCommand,
  RpcResponse,
  RpcSessionState,
  SessionSummary,
  AgentEvent,
  promptCmd,
  steerCmd,
  followUpCmd,
} from "./types.js";
import {
  promptCmd as promptC,
  steerCmd as steerC,
  followUpCmd as followUpC,
} from "./types.js";

export type EventListener = (event: AgentEvent) => void;

export class RpcClient {
  private requestId = 0;
  private eventListeners: EventListener[] = [];
  private connected = false;

  // For TCP connections
  private host: string;
  private port: number;
  private path = "/";
  private useTLS = false;

  // For Unix socket
  private socketPath: string | null = null;

  constructor(baseUrl = "http://localhost:7890") {
    const url = new URL(baseUrl);
    this.host = url.hostname;
    this.port = parseInt(url.port) || (url.protocol === "https:" ? 443 : 80);
    this.path = url.pathname;
    this.useTLS = url.protocol === "https:";
    this.socketPath = process.env.XIHU_SOCKET ?? null;
  }

  isConnected(): boolean {
    return this.connected;
  }

  subscribe(listener: EventListener): () => void {
    this.eventListeners.push(listener);
    return () => {
      this.eventListeners = this.eventListeners.filter((l) => l !== listener);
    };
  }

  private emit(event: AgentEvent): void {
    for (const listener of this.eventListeners) {
      try {
        listener(event);
      } catch {
        // Ignore listener errors
      }
    }
  }

  private async request(body: string): Promise<string> {
    return new Promise((resolve, reject) => {
      const options: http.RequestOptions = {
        socketPath: this.socketPath ?? undefined,
        path: this.path,
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(body),
        },
      };

      const transport = this.useTLS ? https : http;
      const req = transport.request(options, (res) => {
        let data = "";
        res.on("data", (chunk) => (data += chunk));
        res.on("end", () => {
          this.connected = true;
          resolve(data);
        });
      });

      req.on("error", reject);
      req.write(body);
      req.end();
    });
  }

  private async send(cmd: RpcCommand): Promise<RpcResponse> {
    const full = { ...cmd, id: String(++this.requestId) };
    const body = JSON.stringify(full);
    const raw = await this.request(body);
    return JSON.parse(raw) as RpcResponse;
  }

  private async call<T>(type: string, cmd: RpcCommand): Promise<T> {
    const resp = await this.send(cmd);
    if (!resp.success) {
      throw new Error(resp.error ?? "unknown error");
    }
    return resp.data as T;
  }

  // ─── Prompting ───────────────────────────────────────────────────────────

  async prompt(message: string, images?: RpcCommand["images"], streamingBehavior?: "steer" | "followUp"): Promise<void> {
    await this.call("prompt", { type: "prompt", message, images, streamingBehavior });
  }

  async steer(message: string): Promise<void> {
    await this.call("steer", steerC(message));
  }

  async followUp(message: string): Promise<void> {
    await this.call("follow_up", followUpC(message));
  }

  async abort(): Promise<void> {
    await this.call("abort", { type: "abort" });
  }

  async newSession(): Promise<{ cancelled: boolean }> {
    return this.call("new_session", { type: "new_session" });
  }

  // ─── State ───────────────────────────────────────────────────────────────

  async getState(): Promise<RpcSessionState> {
    return this.call<RpcSessionState>("get_state", { type: "get_state" });
  }

  async getMessages(): Promise<{ messages: unknown[] }> {
    return this.call("get_messages", { type: "get_messages" });
  }

  // ─── Model ──────────────────────────────────────────────────────────────

  async setModel(provider: string, modelId: string): Promise<void> {
    await this.call("set_model", { type: "set_model", provider, modelId });
  }

  async cycleModel(): Promise<{ model: string; thinkingLevel: string; isScoped: boolean } | null> {
    return this.call("cycle_model", { type: "cycle_model" });
  }

  async getAvailableModels(): Promise<{ models: string[] }> {
    return this.call("get_available_models", { type: "get_available_models" });
  }

  // ─── Thinking ─────────────────────────────────────────────────────────────

  async setThinkingLevel(level: RpcCommand["level"]): Promise<void> {
    await this.call("set_thinking_level", { type: "set_thinking_level", level });
  }

  async cycleThinkingLevel(): Promise<{ level: string } | null> {
    return this.call("cycle_thinking_level", { type: "cycle_thinking_level" });
  }

  // ─── Queue Modes ─────────────────────────────────────────────────────────

  async setSteeringMode(mode: "all" | "one-at-a-time"): Promise<void> {
    await this.call("set_steering_mode", { type: "set_steering_mode", mode });
  }

  async setFollowUpMode(mode: "all" | "one-at-a-time"): Promise<void> {
    await this.call("set_follow_up_mode", { type: "set_follow_up_mode", mode });
  }

  // ─── Compaction ─────────────────────────────────────────────────────────

  async compact(customInstructions?: string): Promise<string> {
    return this.call("compact", { type: "compact", customInstructions });
  }

  async setAutoCompaction(enabled: boolean): Promise<void> {
    await this.call("set_auto_compaction", { type: "set_auto_compaction", enabled });
  }

  // ─── Retry ──────────────────────────────────────────────────────────────

  async setAutoRetry(enabled: boolean): Promise<void> {
    await this.call("set_auto_retry", { type: "set_auto_retry", enabled });
  }

  async abortRetry(): Promise<void> {
    await this.call("abort_retry", { type: "abort_retry" });
  }

  // ─── Bash ───────────────────────────────────────────────────────────────

  async bash(command: string): Promise<unknown> {
    return this.call("bash", { type: "bash", command });
  }

  async abortBash(): Promise<void> {
    await this.call("abort_bash", { type: "abort_bash" });
  }

  // ─── Session ─────────────────────────────────────────────────────────────

  async getSessionStats(): Promise<unknown> {
    return this.call("get_session_stats", { type: "get_session_stats" });
  }

  async exportHtml(outputPath?: string): Promise<{ path: string }> {
    return this.call("export_html", { type: "export_html", outputPath });
  }

  async switchSession(sessionPath: string): Promise<{ cancelled: boolean }> {
    return this.call("switch_session", { type: "switch_session", sessionPath });
  }

  async fork(entryId: string): Promise<{ text: string; cancelled: boolean }> {
    return this.call("fork", { type: "fork", entryId });
  }

  async clone(): Promise<{ cancelled: boolean }> {
    return this.call("clone", { type: "clone" });
  }

  async getForkMessages(): Promise<{ messages: unknown[] }> {
    return this.call("get_fork_messages", { type: "get_fork_messages" });
  }

  async getLastAssistantText(): Promise<{ text: string | null }> {
    return this.call("get_last_assistant_text", { type: "get_last_assistant_text" });
  }

  async setSessionName(name: string): Promise<void> {
    await this.call("set_session_name", { type: "set_session_name", name });
  }

  // ─── Session Management ─────────────────────────────────────────────────

  async listSessions(): Promise<{ sessions: SessionSummary[] }> {
    return this.call("list_sessions", { type: "list_sessions" });
  }

  async deleteSession(sessionId: string): Promise<{ deleted: boolean }> {
    return this.call("delete_session", { type: "delete_session", sessionId });
  }

  // ─── Commands ────────────────────────────────────────────────────────────

  async getCommands(): Promise<{ commands: unknown[] }> {
    return this.call("get_commands", { type: "get_commands" });
  }
}
