/**
 * gRPC client for xihu Agent.
 * Uses @grpc/grpc-js with proto descriptor.
 * Only supports gRPC (no JSON-RPC or Unix socket).
 */

import * as grpc from "@grpc/grpc-js";
import * as protoLoader from "@grpc/proto-loader";
import type {
  RpcCommand,
  RpcSessionState,
  SessionSummary,
  AgentEvent,
} from "./types.js";

export type EventListener = (event: AgentEvent) => void;

// Load proto descriptor
const PROTO_PATH = process.env.XIHU_PROTO_PATH ?? "/Users/geilige/xihu/proto/proto/xihu.proto";

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
    this.client = new proto.XihuAgent(address, credentials);
  }

  // ─── Session Management ───────────────────────────────────────────────

  getCurrentSessionId(): string {
    return this.currentSessionId;
  }

  setCurrentSessionId(sessionId: string): void {
    this.currentSessionId = sessionId;
  }

  // ─── Event Streaming ─────────────────────────────────────────────────

  connectEvents(): void {
    if (this.streamCall) return;

    this.streamCall = this.client.StreamEvents({
      session_id: this.currentSessionId,
    });
    
    this.streamCall.on("data", (response: any) => {
      try {
        const event: AgentEvent = {
          type: response.type || "message",
          ...(typeof response.data === "string" ? JSON.parse(response.data) : response.data),
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

    this.streamCall.on("end", () => {
      this.streamCall = null;
      // Reconnect after delay
      setTimeout(() => this.connectEvents(), 2000);
    });

    this.streamCall.on("error", (err: Error) => {
      console.error("Stream error:", err);
      this.streamCall = null;
    });

    this.connected = true;
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
  }

  // ─── RPC Call Helper ─────────────────────────────────────────────────

  private async call(type: string, cmd: Partial<RpcCommand>): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const request = {
        id: String(Date.now()),
        type,
        sessionId: this.currentSessionId || undefined,
        ...cmd,
      };

      this.client.ExecuteCommand(request, (err: Error | null, response: any) => {
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
  }

  // ─── Session Management RPC Methods ──────────────────────────────────

  async newSession(): Promise<{ sessionId?: string; cancelled: boolean }> {
    const result = await this.call("new_session", {}) as any;
    if (result?.sessionId) {
      this.currentSessionId = result.sessionId;
    }
    return result || { cancelled: false };
  }

  async switchSession(sessionId: string): Promise<{ cancelled: boolean }> {
    const result = await this.call("switch_session", { sessionId }) as any;
    if (result && !result.cancelled) {
      this.currentSessionId = sessionId;
    }
    return result || { cancelled: false };
  }

  async fork(entryId: string): Promise<{ text: string; cancelled: boolean }> {
    const result = await this.call("fork", { entryId }) as any;
    if (result?.sessionId) {
      this.currentSessionId = result.sessionId;
    }
    return result || { text: "", cancelled: true };
  }

  async clone(): Promise<{ cancelled: boolean }> {
    const result = await this.call("clone", {}) as any;
    if (result?.sessionId) {
      this.currentSessionId = result.sessionId;
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

  async getAvailableModels(): Promise<{ models: string[] }> {
    return this.call("get_available_models", {}) as Promise<{ models: string[] }>;
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
}
