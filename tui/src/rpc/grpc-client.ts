/**
 * gRPC client for FutureAgent.
 * Uses @grpc/grpc-js with proto descriptor.
 * Only supports gRPC (no JSON-RPC or Unix socket).
 */

// Must import proto-setup BEFORE any gRPC modules — it injects Long globally
// for protobufjs, which does a dynamic global lookup in bun build --compile.
import "./proto-setup.js";

import * as grpc from "@grpc/grpc-js";
import { loadGrpcAgentProto, responseData, streamEventData } from "@future-os/rpc";
import type {
  RpcCommand,
  RpcSessionState,
  SessionSummary,
  AgentEvent,
  ThinkingLevel,
} from "./types.js";

export type EventListener = (event: AgentEvent) => void;

/// Default gRPC deadline (seconds).  Used by every unary RPC call; any
/// single call that takes longer is treated as a timeout.  30 s covers the
/// slowest legitimate agent operation (large compaction + model response).
const GRPC_DEADLINE_SEC = 30;

// The proto schema and its grpc-js loader live in the shared wire-contract
// package (@future-os/rpc) — single source of truth for every TS client.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const proto: any = loadGrpcAgentProto();

// ─── RPC Client ─────────────────────────────────────────────────────────

export type ConnectionChangeListener = (connected: boolean) => void;

export class GrpcClient {
  private client: any;
  private readonly address: string;
  private eventListeners: EventListener[] = [];
  private streamCall: any = null;
  private connected = false;
  private currentSessionId: string = "";
  private activeRunId: string | null = null;
  private runs = new Map<string, "queued" | "running" | "terminal">();
  private agentInstanceId: string | null = null;
  private lostQueuedRunIds: string[] = [];
  /// Resolved when the event stream delivers the first event (or the stream
  /// fails).  Eliminates the busy-wait poll loop in call() — callers await
  /// this instead of spinning every 100ms.
  private connectPromise: Promise<boolean> | null = null;
  private connectResolve: ((value: boolean) => void) | null = null;
  private connectionChangeListeners: ConnectionChangeListener[] = [];

  // ─── Reconnect bookkeeping ───────────────────────────────────────────

  /// Counts consecutive tryConnect() failures during reconnect polling.
  /// After 3 failures the gRPC channel is likely stuck in TRANSIENT_FAILURE
  /// and we recreate the client to force a fresh channel.
  private reconnectFailures = 0;
  /// Periodic health-check that calls tryConnect() to detect silent
  /// disconnections (agent process killed without the stream emitting
  /// an error/end event).
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;

  constructor(address = "localhost:50051") {
    this.address = address;
    this.client = this.createClient();
  }

  private createClient(): any {
    const credentials = grpc.credentials.createInsecure();
    return new proto.FutureAgent(this.address, credentials);
  }

  // ─── Connection state callbacks ──────────────────────────────────────

  onConnectionChange(listener: ConnectionChangeListener): () => void {
    this.connectionChangeListeners.push(listener);
    return () => {
      this.connectionChangeListeners = this.connectionChangeListeners.filter((l) => l !== listener);
    };
  }

  private notifyConnectionChange(connected: boolean): void {
    for (const listener of this.connectionChangeListeners) {
      try { listener(connected); } catch { /* ignore */ }
    }
  }

  // ─── Session Management ───────────────────────────────────────────────

  getCurrentSessionId(): string {
    return this.currentSessionId;
  }

  setCurrentSessionId(sessionId: string): void {
    this.currentSessionId = sessionId;
    this.activeRunId = null;
    this.runs.clear();
  }

  // ─── Event Streaming ─────────────────────────────────────────────────

  /// Lightweight connectivity check — sends a simple RPC (list_models) without
  /// requiring a session or event-stream handshake.  Returns true if the agent
  /// is reachable, false otherwise.  Times out after 3 s.
  async tryConnect(): Promise<boolean> {
    try {
      const request = { id: String(Date.now()), type: "list_models" };
      const deadline = new Date();
      deadline.setSeconds(deadline.getSeconds() + 3);
      await new Promise<void>((resolve, reject) => {
        this.client.ExecuteCommand(request, { deadline }, (err: Error | null, _response: any) => {
          if (err) reject(err);
          else resolve();
        });
      });
      return true;
    } catch {
      return false;
    }
  }

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

    // Never subscribe without a session ID — an empty session_id may cause
    // the server to broadcast events from ALL sessions, leaking GUI streams
    // into the TUI.
    if (!this.currentSessionId) {
      this.connected = false;
      return;
    }

    // Create a fresh connection promise — resolved once the first event
    // arrives (connected=true) or the stream fails.  Eliminates the busy-wait
    // poll loop in call().
    this.connectPromise = new Promise((resolve) => {
      this.connectResolve = resolve;
    });

    const scheduleReconnect = () => {
      if (!this.reconnectTimer) {
        const wasConnected = this.connected;
        this.connected = false;
        this.stopHeartbeat(); // stop health-checks while disconnected
        this.connectResolve?.(false); // let call() proceed with timeout
        if (wasConnected) {
          this.notifyConnectionChange(false);
        }
        this.reconnectTimer = setTimeout(async () => {
          this.reconnectTimer = null;
          // Poll via unary RPC (3 s deadline) instead of blindly calling
          // StreamEvents.  When the agent is down the gRPC channel may
          // return a stream object that never emits data/error/end — the
          // reconnect loop would stall forever.  tryConnect() fails fast
          // when the server is unreachable, so we only attempt the
          // streaming handshake when the agent is confirmed alive.
          if (await this.tryConnect()) {
            this.reconnectFailures = 0;
            this.connectEvents();
          } else {
            this.reconnectFailures++;
            // After 3 consecutive failures the gRPC channel is likely stuck
            // in TRANSIENT_FAILURE — recreate the client for a fresh channel.
            if (this.reconnectFailures >= 3) {
              this.reconnectFailures = 0;
              this.client = this.createClient();
            }
            scheduleReconnect();
          }
        }, 1000);
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

    // Watchdog: if StreamEvents created a stream but no data arrives
    // within 5 s the underlying channel is likely stuck.  Cancel the
    // stream so the reconnect loop can try again from scratch.
    let connectWatchdog: ReturnType<typeof setTimeout> | null = setTimeout(() => {
      connectWatchdog = null;
      if (this.streamCall === call) {
        this.streamCall = null;
        call.cancel();
        scheduleReconnect();
      }
    }, 5000);

    call.on("data", (response: any) => {
      // Discard events from a stale stream — when connectEvents() cancels an
      // old stream and creates a new one, buffered data events from the old
      // stream can still arrive asynchronously.  Without this guard those
      // events leak into the new session's chat, corrupting messages and
      // leaving the streaming indicator stuck on.
      if (this.streamCall !== call) return;
      if (connectWatchdog) { clearTimeout(connectWatchdog); connectWatchdog = null; }
      if (!this.connected) {
        this.connected = true;
        this.connectResolve?.(true);
        this.connectResolve = null;
        this.notifyConnectionChange(true);
        this.startHeartbeat();
      }
      try {
        // Shared wire decode: typed payload when present, JSON `data`
        // fallback; the injected `type` key is dropped either way.
        const rest = streamEventData(response);
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
          ...rest,
        };
        if (event.runId && event.type === "agent_start") {
          this.activeRunId = event.runId;
          this.runs.set(event.runId, "running");
        } else if (event.runId && event.type === "agent_end") {
          this.runs.set(event.runId, "terminal");
          if (this.activeRunId === event.runId) this.activeRunId = null;
        }

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
      if (connectWatchdog) { clearTimeout(connectWatchdog); connectWatchdog = null; }
      if (this.streamCall === call) {
        this.streamCall = null;
        scheduleReconnect();
      }
    });

    call.on("error", (_err: Error) => {
      if (connectWatchdog) { clearTimeout(connectWatchdog); connectWatchdog = null; }
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
    this.stopHeartbeat();
    this.streamCall?.cancel();
    this.streamCall = null;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    this.connected = false;
  }

  // ─── Heartbeat ──────────────────────────────────────────────────────

  /// Start a periodic health-check that calls tryConnect() every 10 s.
  /// If the agent is unreachable but the stream didn't emit error/end
  /// (e.g. the process was SIGKILL'd), the heartbeat detects the silent
  /// disconnection and triggers the reconnect loop.
  private startHeartbeat(): void {
    this.stopHeartbeat();
    this.heartbeatTimer = setInterval(async () => {
      if (!this.connected) return;
      try {
        const alive = await this.tryConnect();
        if (!alive && this.connected) {
          // Agent is down — cancel the stale stream and kick off reconnect.
          this.streamCall?.cancel();
          this.streamCall = null;
          const wasConnected = this.connected;
          this.connected = false;
          this.connectResolve?.(false);
          if (wasConnected) {
            this.notifyConnectionChange(false);
          }
          this.connectEvents();
        }
      } catch {
        // tryConnect threw — treat as unreachable.
      }
    }, 10_000);
  }

  private stopHeartbeat(): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  // ─── RPC Call Helper ─────────────────────────────────────────────────

  private async call(type: string, cmd: Partial<RpcCommand>, retry = true): Promise<unknown> {
    // Wait for connection if not yet connected (first call or reconnecting).
    // Await the connection promise (resolved on first stream event) instead of
    // a busy-wait poll loop — avoids burning 100ms-interval CPU ticks.
    if (!this.connected) {
      if (!this.reconnectTimer) {
        this.connectEvents();
      }
      // Wait up to 5 s for the event stream to deliver its first frame.
      const timeout = new Promise<boolean>((r) => setTimeout(() => r(false), 5000));
      await Promise.race([this.connectPromise, timeout]);
    }

    const doCall = (): Promise<unknown> => new Promise((resolve, reject) => {
      const request = {
        id: String(Date.now()),
        type,
        sessionId: this.currentSessionId || undefined,
        ...cmd,
      };

      const deadline = new Date();
      deadline.setSeconds(deadline.getSeconds() + GRPC_DEADLINE_SEC);
      this.client.ExecuteCommand(request, { deadline }, (err: Error | null, response: any) => {
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
      });
    });

    try {
      return await doCall();
    } catch (err: any) {
      // On transport error, trigger reconnect so stream comes back.
      // Don't retry the call itself — for non-idempotent commands like
      // 'prompt', the request may have already reached the agent and
      // we'd create a duplicate. The stream will deliver events either way.
      //
      // IMPORTANT: when the stream already reports we're connected (data
      // arrived via StreamEvents), a transient unary RPC failure must NOT
      // override `connected` and tear down the working stream.  The stream
      // may be alive while the unary channel is still warming up after a
      // reconnect.  Let the heartbeat or stream error/end handlers detect
      // a real disconnection instead.
      const msg = err?.message || String(err);
      const isTransport = msg.includes("transport") || msg.includes("14 UNAVAILABLE")
        || msg.includes("Connect Failed") || msg.includes("ECONNREFUSED");
      if (isTransport) {
        if (!this.connected) {
          // Stream also reports disconnected — full reconnect needed.
          this.connectEvents();
        }
        // else: stream is alive, transient unary failure — don't tear it down.
      }
      throw err;
    }
  }

  // ─── Session Management RPC Methods ──────────────────────────────────

  async newSession(opts?: { cwd?: string; modelId?: string; level?: ThinkingLevel }): Promise<{ sessionId?: string; cancelled: boolean }> {
    const result = await this.call("new_session", {
      // Clear sessionId so the agent generates a fresh ID instead of
      // reusing the current session's ID (which would load old entries).
      sessionId: undefined as any,
      cwd: opts?.cwd || process.cwd(),
      modelId: opts?.modelId,
      level: opts?.level,
      createdBy: "tui",
    }) as any;
    if (result?.sessionId) {
      this.setCurrentSessionId(result.sessionId);
      this.connectEvents();
    }
    return result || { cancelled: false };
  }

  async switchSession(sessionId: string): Promise<{ cancelled: boolean }> {
    const result = await this.call("switch_session", { sessionId }) as any;
    if (result && !result.cancelled) {
      this.setCurrentSessionId(sessionId);
      this.connectEvents();
    }
    return result || { cancelled: false };
  }

  async fork(entryId: string): Promise<{ text: string; cancelled: boolean }> {
    const result = await this.call("fork", { entryId }) as any;
    if (result?.sessionId) {
      this.setCurrentSessionId(result.sessionId);
      this.connectEvents();
    }
    return result || { text: "", cancelled: true };
  }

  async clone(): Promise<{ cancelled: boolean }> {
    const result = await this.call("clone", {}) as any;
    if (result?.sessionId) {
      this.setCurrentSessionId(result.sessionId);
      this.connectEvents();
    }
    return result || { cancelled: true };
  }

  async getForkMessages(): Promise<{ messages: unknown[] }> {
    return this.call("get_fork_messages", {}) as Promise<{ messages: unknown[] }>;
  }

  async setSessionName(name: string): Promise<void> {
    await this.call("set_session_name", { name });
  }

  async listSessions(): Promise<{ sessions: SessionSummary[] }> {
    return this.call("list_sessions", {}) as Promise<{ sessions: SessionSummary[] }>;
  }

  // ─── Core RPC Methods ────────────────────────────────────────────────

  async prompt(
    message: string,
    images?: RpcCommand["images"],
    busyPolicy: RpcCommand["busyPolicy"] = "reject_if_busy",
  ): Promise<import("./types.js").RunAck> {
    const requestId = crypto.randomUUID();
    const ack = await this.call("prompt", {
      message,
      images,
      busyPolicy,
      requestedRunId: `run_${crypto.randomUUID().replaceAll("-", "")}`,
      clientRequestId: `request_${requestId.replaceAll("-", "")}`,
    }) as import("./types.js").RunAck;
    if (ack.accepted_state === "running") {
      this.activeRunId = ack.run_id;
      this.runs.set(ack.run_id, "running");
    } else if (ack.accepted_state === "queued") {
      this.runs.set(ack.run_id, "queued");
    }
    return ack;
  }

  async abort(): Promise<void> {
    await this.call("abort", { runId: this.activeRunId || undefined });
  }

  async cancelQueuedRun(runId: string): Promise<void> {
    await this.call("cancel_queued_run", { runId });
    this.runs.delete(runId);
  }

  /** Queued work is intentionally memory-only. Surface losses after restart
   * rather than leaving stale queued bubbles indefinitely. */
  takeLostQueuedRunIds(): string[] {
    const lost = this.lostQueuedRunIds;
    this.lostQueuedRunIds = [];
    return lost;
  }

  async getState(): Promise<RpcSessionState> {
    const state = await this.call("get_state", {}) as RpcSessionState;
    if (this.agentInstanceId && state.agentInstanceId && this.agentInstanceId !== state.agentInstanceId) {
      this.lostQueuedRunIds.push(...[...this.runs]
        .filter(([, status]) => status === "queued")
        .map(([runId]) => runId));
    }
    if (state.agentInstanceId) this.agentInstanceId = state.agentInstanceId;
    this.runs.clear();
    if (state.activeRun?.runId) {
      this.activeRunId = state.activeRun.runId;
      this.runs.set(state.activeRun.runId, "running");
    } else {
      this.activeRunId = null;
    }
    for (const queued of state.queuedRuns ?? []) this.runs.set(queued.runId, "queued");
    return state;
  }

  hasRunningRun(): boolean {
    return [...this.runs.values()].some((state) => state === "running");
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

  async setThinkingLevel(level: RpcCommand["level"]): Promise<void> {
    await this.call("set_thinking_level", { level });
  }

  async cycleThinkingLevel(): Promise<{ level: string } | null> {
    return this.call("cycle_thinking_level", {}) as Promise<{ level: string } | null>;
  }

  async compact(customInstructions?: string): Promise<string> {
    return this.call("compact", { customInstructions }) as Promise<string>;
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

  async reloadConfig(): Promise<{ skills: string[]; contextFiles: string[] }> {
    return this.call("reload_config", {}) as Promise<{ skills: string[]; contextFiles: string[] }>;
  }
}
