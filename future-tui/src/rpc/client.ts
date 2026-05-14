/**
 * JSON-RPC client for future_tui Agent.
 * Supports both TCP (http://host:port) and Unix socket.
 * Also handles SSE event streaming.
 */

import http from "node:http";
import https from "node:https";
import { URL } from "node:url";
import type {
  RpcCommand,
  RpcSessionState,
  SessionSummary,
  AgentEvent,
} from "./types.js";

export type EventListener = (event: AgentEvent) => void;

// ─── RPC Client ─────────────────────────────────────────────────────────

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

  // SSE connection
  private eventsource: SSEConnection | null = null;

  constructor(baseUrl = "http://localhost:7890") {
    const url = new URL(baseUrl);
    this.host = url.hostname;
    this.port = parseInt(url.port) || (url.protocol === "https:" ? 443 : 80);
    this.path = url.pathname;
    this.useTLS = url.protocol === "https:";
    this.socketPath = process.env.XIHU_SOCKET ?? null;
  }

  // ─── SSE Events ──────────────────────────────────────────────────────

  /**
   * Connect to the SSE event stream for real-time agent events.
   * Automatically called when subscribing to events.
   */
  connectEvents(): void {
    if (this.eventsource) return;

    this.eventsource = new SSEConnection(
      this.socketPath ?? undefined,
      this.host !== "localhost" ? `${this.useTLS ? "https" : "http"}://${this.host}:${this.port}` : undefined,
      (event) => {
        for (const listener of this.eventListeners) {
          try {
            listener(event);
          } catch {
            // Ignore listener errors
          }
        }
      }
    );
  }

  isConnected(): boolean {
    return this.connected;
  }

  subscribe(listener: EventListener): () => void {
    // Ensure SSE is connected
    this.connectEvents();
    this.eventListeners.push(listener);
    return () => {
      this.eventListeners = this.eventListeners.filter((l) => l !== listener);
    };
  }

  disconnect(): void {
    this.eventsource?.close();
    this.eventsource = null;
  }

  // ─── HTTP Request ───────────────────────────────────────────────────

  private async request(body: string): Promise<string> {
    return new Promise((resolve, reject) => {
      const options: http.RequestOptions = {
        socketPath: this.socketPath ?? undefined,
        hostname: this.socketPath ? undefined : this.host,
        port: this.socketPath ? undefined : this.port,
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

  private async send(cmd: RpcCommand): Promise<unknown> {
    const full = { ...cmd, id: String(++this.requestId) };
    const body = JSON.stringify(full);
    const raw = await this.request(body);
    return JSON.parse(raw);
  }

  private async call<T>(type: string, cmd: RpcCommand): Promise<T> {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const resp = (await this.send(cmd)) as any;
    if (!resp.success) {
      throw new Error(resp.error ?? "unknown error");
    }
    return resp.data as T;
  }

  // ─── RPC Methods ─────────────────────────────────────────────────────

  async prompt(message: string, images?: RpcCommand["images"], streamingBehavior?: "steer" | "followUp"): Promise<void> {
    await this.call("prompt", { type: "prompt", message, images, streamingBehavior });
  }

  async steer(message: string): Promise<void> {
    await this.call("steer", { type: "steer", message });
  }

  async followUp(message: string): Promise<void> {
    await this.call("follow_up", { type: "follow_up", message });
  }

  async abort(): Promise<void> {
    await this.call("abort", { type: "abort" });
  }

  async newSession(): Promise<{ cancelled: boolean }> {
    return this.call("new_session", { type: "new_session" });
  }

  async getState(): Promise<RpcSessionState> {
    return this.call<RpcSessionState>("get_state", { type: "get_state" });
  }

  async getMessages(): Promise<{ messages: unknown[] }> {
    return this.call("get_messages", { type: "get_messages" });
  }

  async setModel(modelId: string): Promise<void> {
    await this.call("set_model", { type: "set_model", modelId });
  }

  async cycleModel(): Promise<{ model: string; thinkingLevel: string; isScoped: boolean } | null> {
    return this.call("cycle_model", { type: "cycle_model" });
  }

  async getAvailableModels(): Promise<{ models: string[] }> {
    return this.call("get_available_models", { type: "get_available_models" });
  }

  async setThinkingLevel(level: RpcCommand["level"]): Promise<void> {
    await this.call("set_thinking_level", { type: "set_thinking_level", level });
  }

  async cycleThinkingLevel(): Promise<{ level: string } | null> {
    return this.call("cycle_thinking_level", { type: "cycle_thinking_level" });
  }

  async setSteeringMode(mode: "all" | "one-at-a-time"): Promise<void> {
    await this.call("set_steering_mode", { type: "set_steering_mode", mode });
  }

  async setFollowUpMode(mode: "all" | "one-at-a-time"): Promise<void> {
    await this.call("set_follow_up_mode", { type: "set_follow_up_mode", mode });
  }

  async compact(customInstructions?: string): Promise<string> {
    return this.call("compact", { type: "compact", customInstructions });
  }

  async setAutoCompaction(enabled: boolean): Promise<void> {
    await this.call("set_auto_compaction", { type: "set_auto_compaction", enabled });
  }

  async setAutoRetry(enabled: boolean): Promise<void> {
    await this.call("set_auto_retry", { type: "set_auto_retry", enabled });
  }

  async abortRetry(): Promise<void> {
    await this.call("abort_retry", { type: "abort_retry" });
  }

  async bash(command: string): Promise<unknown> {
    return this.call("bash", { type: "bash", command });
  }

  async abortBash(): Promise<void> {
    await this.call("abort_bash", { type: "abort_bash" });
  }

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

  async listSessions(): Promise<{ sessions: SessionSummary[] }> {
    return this.call("list_sessions", { type: "list_sessions" });
  }

  async deleteSession(sessionId: string): Promise<{ deleted: boolean }> {
    return this.call("delete_session", { type: "delete_session", sessionId });
  }

  async getCommands(): Promise<{ commands: unknown[] }> {
    return this.call("get_commands", { type: "get_commands" });
  }
}

// ─── SSE Connection ─────────────────────────────────────────────────────

/**
 * Server-Sent Events client using Node.js HTTP.
 * Connects to GET /events on the future_tui server and parses SSE data.
 */
class SSEConnection {
  private req: http.ClientRequest | null = null;
  private buffer = "";

  constructor(
    private socketPath: string | undefined,
    private baseUrl: string | undefined,
    private onEvent: (event: AgentEvent) => void
  ) {
    this.connect();
  }

  private connect(): void {
    const options: http.RequestOptions = {
      socketPath: this.socketPath,
      hostname: this.socketPath ? undefined : (this.baseUrl ? new URL(this.baseUrl).hostname : "localhost"),
      port: this.socketPath ? undefined : (this.baseUrl ? new URL(this.baseUrl).port : 7890),
      path: "/events",
      method: "GET",
      headers: {
        "Accept": "text/event-stream",
        "Cache-Control": "no-cache",
      },
    };

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const transport = this.baseUrl?.startsWith("https") ? https : http;
    this.req = transport.request(options, (res) => {
      res.on("data", (chunk: string) => {
        this.buffer += chunk;
        this.processBuffer();
      });
      res.on("end", () => {
        // Connection closed
      });
    });

    this.req.on("error", () => {
      // Retry connection after delay
      setTimeout(() => this.connect(), 2000);
    });

    this.req.end();
  }

  private processBuffer(): void {
    // SSE format: "event: TYPE\ndata: JSON\n\n"
    // Extract complete events (delimited by \n\n)
    const events = this.buffer.split("\n\n");
    this.buffer = events.pop() ?? ""; // Keep incomplete last chunk

    for (const raw of events) {
      this.parseEvent(raw);
    }
  }

  private parseEvent(raw: string): void {
    let eventType = "message";
    let data = "";

    for (const line of raw.split("\n")) {
      if (line.startsWith("event: ")) {
        eventType = line.slice(7).trim();
      } else if (line.startsWith("data: ")) {
        data = line.slice(6).trim();
      }
    }

    if (!data) return;

    try {
      const event = JSON.parse(data) as AgentEvent;
      // Override type with SSE event type
      if (eventType !== "message") {
        event.type = eventType;
      }
      this.onEvent(event);
    } catch {
      // Ignore parse errors
    }
  }

  close(): void {
    this.req?.destroy();
    this.req = null;
  }
}
