import type { NatsConnection } from "nats.ws";
import { connect, ErrorCode, jwtAuthenticator } from "nats.ws";
import { fromSeed } from "nkeys.js";
import { ensureFreshCredentials, refreshCredentials } from "./pairing";
import { jwtExpiry, randomId, encodeBase64Url } from "./codec";
import { backoffDelayMs, classifyError, transition, type ConnectionState } from "./connectionState";
import { handshakeTranscript, type HandshakeChallenge, verifyDesktopChallenge } from "./handshake";
import type {
  Presence,
  PresenceSession,
  RemoteCommand,
  RemoteCredentials,
  RemoteWorkspace,
  RpcResponse,
  StreamEvent,
} from "./types";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const HANDSHAKE_PROTOCOL_VERSION = 1;
const FOREGROUND_PROBE_TIMEOUT_MS = 4_000;

export type RecoveryReason =
  "foreground" | "network-restored" | "network-changed" | "request-failure";

interface HandshakeConfirmation {
  confirmed: boolean;
  pairId: string;
  desktopId: string;
  bridgeInstanceId: string;
  deviceId: string;
  desktopNonce: string;
  presence: Presence;
  features?: string[];
}

export interface RemoteClientCallbacks {
  onCredentials(credentials: RemoteCredentials): void;
  onEvent(event: StreamEvent, sessionId: string): void;
  onEventDecodeFailure(sessionId: string, error: Error): void;
  onPresence(presence: Presence): void;
  onSessions(sessions: PresenceSession[]): void;
  onWorkspaces(workspaces: RemoteWorkspace[]): void;
  onFeatures(features: string[]): void;
  onConnectionState(state: ConnectionState): void;
  onReconnected(): void;
  onError(error: Error): void;
}

function decodeJson<T>(data: Uint8Array): T {
  return JSON.parse(decoder.decode(data)) as T;
}

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  try {
    return await Promise.race([
      promise,
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error("network_probe_timeout")), timeoutMs);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

export class RemoteClient {
  private connection: NatsConnection | null = null;
  private credentials: RemoteCredentials;
  private refreshTimer: ReturnType<typeof setTimeout> | null = null;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private retryAttempt = 0;
  /**
   * Consecutive refreshable-auth failures. A single auth failure rotates the
   * JWT once; if the NEXT attempt also fails with auth (the token service is
   * healthy but the handshake keeps failing), we must not loop refresh→open→
   * handshake at one full RTT per cycle with no backoff. After the first
   * refresh, subsequent auth failures fall through to the shared backoff
   * timer; a successful open resets the counter.
   */
  private authRetryCount = 0;
  private generation = 0;
  private stopped = false;
  private appActive = true;
  private networkAvailable = true;
  private recoveryPromise: Promise<void> | null = null;
  private failedGeneration: number | null = null;
  private state: ConnectionState = "unpaired";
  private confirmedBridgeInstanceId = "";
  private handshakePromise: Promise<HandshakeConfirmation> | null = null;
  /** Guards against overlapping token refresh (timer + auth failure racing). */
  private refreshInFlight = false;
  private downloadWaiters = new Map<
    string,
    {
      resolve(data: Uint8Array): void;
      reject(error: Error): void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();

  private rejectDownloadWaiters(reason: string): void {
    for (const waiter of this.downloadWaiters.values()) {
      clearTimeout(waiter.timer);
      waiter.reject(new Error(reason));
    }
    this.downloadWaiters.clear();
  }

  constructor(
    credentials: RemoteCredentials,
    private readonly callbacks: RemoteClientCallbacks,
  ) {
    this.credentials = credentials;
  }

  /**
   * Establish (or re-establish) the live connection. The attempt runs in the
   * background: on a transport failure this arms the backoff timer and returns,
   * so a slow attempt never blocks its caller (H4's "close() then reconnect
   * 1s later" storm is structurally impossible — close never arms a timer, and
   * the retry timer is owned by exactly one place, here).
   */
  async open(): Promise<void> {
    if (this.stopped || !this.appActive || !this.networkAvailable) return;
    this.signal({ type: "open_started" });
    const generation = ++this.generation;
    try {
      const fresh = await ensureFreshCredentials(this.credentials);
      if (this.stopped || generation !== this.generation) return;
      this.credentials = fresh;
      this.callbacks.onCredentials(fresh);
      await this.connectSocket(generation);
      if (
        this.stopped ||
        !this.appActive ||
        !this.networkAvailable ||
        generation !== this.generation
      )
        return;
      this.scheduleRefresh();
    } catch (error) {
      if (this.stopped || generation !== this.generation) return;
      this.handleFailure(error);
    }
  }

  /** Full teardown — never arms a retry, never broadcasts a phase. Idempotent. */
  async close(reason: "UserInitiated" | "Unpair" = "UserInitiated"): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;
    this.clearTimers();
    this.disposeConnection(reason === "UserInitiated" ? "close" : "unpair");
    this.rejectDownloadWaiters("closed");
  }

  /** Stop live iterators and retries while the OS reports no usable network. */
  setNetworkAvailable(available: boolean): void {
    if (this.stopped || available === this.networkAvailable) return;
    this.networkAvailable = available;
    if (available) return;
    this.generation += 1;
    this.clearTimers();
    this.signal({ type: "transport_disconnect" });
    this.disposeConnection("network_unavailable");
  }

  /**
   * Ordinary WebSockets are not a background execution mechanism on either
   * mobile OS. Close deliberately before JavaScript is suspended so foreground
   * recovery starts from a known state instead of inheriting a half-open WSS.
   */
  pauseForBackground(): void {
    if (this.stopped || !this.appActive) return;
    this.appActive = false;
    this.generation += 1;
    this.clearTimers();
    this.signal({ type: "transport_disconnect" });
    this.disposeConnection("background");
  }

  /** Validate after foregrounding, or immediately rebuild after a path change. */
  recoverNow(reason: RecoveryReason): Promise<void> {
    if (this.stopped) return Promise.resolve();
    if (reason === "foreground") this.appActive = true;
    if (!this.appActive) return Promise.resolve();
    if (reason !== "foreground") this.networkAvailable = true;
    if (!this.networkAvailable) return Promise.resolve();
    if (this.recoveryPromise) return this.recoveryPromise;
    const recovery = this.runRecovery(reason).finally(() => {
      if (this.recoveryPromise === recovery) this.recoveryPromise = null;
    });
    this.recoveryPromise = recovery;
    return recovery;
  }

  private async runRecovery(reason: RecoveryReason): Promise<void> {
    const connection = this.connection;
    const generation = this.generation;
    if (
      (reason === "foreground" || reason === "request-failure") &&
      connection &&
      !connection.isClosed()
    ) {
      try {
        // A command timeout can be a slow desktop handler or a half-open WSS.
        // Probe briefly: keep a healthy socket (the stable command id will
        // retrieve the singleflight result), but rebuild a half-open one before
        // the only prepare retry is spent on the same dead path.
        await withTimeout(
          connection.flush(),
          reason === "request-failure" ? 1_000 : FOREGROUND_PROBE_TIMEOUT_MS,
        );
        if (
          !this.stopped &&
          this.networkAvailable &&
          generation === this.generation &&
          connection === this.connection
        ) {
          return;
        }
      } catch {
        // Rebuild below without waiting for NATS's ping budget to expire.
      }
    }
    if (this.stopped || !this.appActive || !this.networkAvailable) return;
    this.generation += 1;
    this.clearTimers();
    this.signal({ type: "transport_disconnect" });
    this.disposeConnection(reason);
    this.retryAttempt = 0;
    await this.open();
  }

  /** Rotate the JWT in place and resume the connection (M1's refreshable class). */
  private async refreshToken(): Promise<void> {
    if (this.refreshInFlight || !this.appActive) return;
    this.refreshInFlight = true;
    try {
      this.signal({ type: "auth_failed" }); // moves to refreshing
      const fresh = await refreshCredentials(this.credentials);
      if (this.stopped) return;
      this.credentials = fresh;
      this.callbacks.onCredentials(fresh);
      this.disposeConnection("refresh");
      // Re-open with the fresh token; the FSM is already in refreshing.
      await this.open();
    } catch (error) {
      if (this.stopped) return;
      // Refresh is attempted ONCE. A repeat failure must not loop straight
      // back into another refresh (M1's infinite retry): a revoked refresh
      // token is terminal, anything else backs off and re-opens, where
      // ensureFreshCredentials will try the refresh again with backoff.
      if (classifyError(error) === "authTerminal") {
        this.clearTimers();
        this.signal({ type: "revoked" });
        this.callbacks.onError(error instanceof Error ? error : new Error(String(error)));
        return;
      }
      this.callbacks.onError(error instanceof Error ? error : new Error(String(error)));
      this.scheduleRetry();
    } finally {
      this.refreshInFlight = false;
    }
  }

  private async connectSocket(generation: number): Promise<void> {
    const seed = encoder.encode(this.credentials.seed);
    let connection: NatsConnection;
    try {
      connection = await connect({
        servers: this.credentials.natsWsUrl,
        inboxPrefix: `p.${this.credentials.pairId}.rep.${this.credentials.deviceId}`,
        authenticator: jwtAuthenticator(this.credentials.userJwt, seed),
      });
    } catch (error) {
      throw errorWithContext(
        `nats_connect_failed (${safeServerLabel(this.credentials.natsWsUrl)})`,
        error,
      );
    }
    if (this.stopped || generation !== this.generation) {
      await connection.close();
      return;
    }
    try {
      let confirmation: HandshakeConfirmation;
      try {
        confirmation = await this.ensureHandshake(connection);
      } catch (error) {
        throw errorWithContext("desktop_handshake_failed", error);
      }
      if (this.stopped || generation !== this.generation) {
        await connection.close();
        return;
      }
      // Dispose any prior connection ONLY after the new one is proven — M10's
      // ghost connection: the old path nulled `this.connection` before the
      // refresh, leaking it on failure.
      const previous = this.connection;
      if (previous && previous !== connection) {
        await previous.close().catch(() => undefined);
      }
      this.connection = connection;
      this.confirmedBridgeInstanceId = confirmation.bridgeInstanceId;
      this.retryAttempt = 0;
      this.authRetryCount = 0;
      this.failedGeneration = null;
      this.callbacks.onPresence(confirmation.presence);
      this.callbacks.onFeatures(confirmation.features ?? []);
      this.subscribeEvents(connection, generation);
      this.subscribeLiveness(connection, generation);
      this.subscribeState(connection, generation);
      this.subscribeTransfers(connection, generation);
      this.watchStatus(connection, generation);
      this.signal({ type: "ready" });
      this.callbacks.onReconnected();
    } catch (error) {
      await connection.close().catch(() => undefined);
      throw error;
    }
  }

  private handleFailure(error: unknown): void {
    const kind = classifyError(error);
    if (kind === "authTerminal") {
      // The device was revoked (M1) — stop every network action and tell the
      // UI to guide the user to re-pair.
      this.clearTimers();
      this.signal({ type: "revoked" });
      this.callbacks.onError(error instanceof Error ? error : new Error(String(error)));
      return;
    }
    if (kind === "fatal") {
      this.clearTimers();
      const failure = error instanceof Error ? error : new Error(String(error));
      this.signal({ type: "fatal", error: failure });
      this.callbacks.onError(failure);
      return;
    }
    if (kind === "auth") {
      // Refreshable — rotate the JWT ONCE, then back off. A handshake that
      // keeps failing after a successful refresh must not spin at one full
      // RTT per cycle (no backoff): the second consecutive auth failure takes
      // the shared retry path instead.
      this.authRetryCount += 1;
      if (this.authRetryCount === 1 && !this.refreshInFlight) {
        void this.refreshToken();
        return;
      }
      this.callbacks.onError(error instanceof Error ? error : new Error(String(error)));
      this.scheduleRetry();
      return;
    }
    this.callbacks.onError(error instanceof Error ? error : new Error(String(error)));
    this.scheduleRetry();
  }

  /** A live generation may lose several subscriptions at once. Reconnect it
   * exactly once so their terminal callbacks cannot reset the retry timer into
   * a reconnect storm. */
  private failGeneration(error: unknown, generation: number): void {
    if (this.stopped || generation !== this.generation || this.failedGeneration === generation) {
      return;
    }
    this.failedGeneration = generation;
    this.handleFailure(error);
  }

  private scheduleRetry(): void {
    this.signal({ type: "open_failed", error: new Error("transport_failed") });
    if (this.stopped || !this.appActive || !this.networkAvailable) return;
    if (this.retryTimer) clearTimeout(this.retryTimer);
    const delay = backoffDelayMs(this.retryAttempt);
    this.retryAttempt += 1;
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      if (this.stopped || !this.appActive || !this.networkAvailable) return;
      void this.open();
    }, delay);
  }

  private clearTimers(): void {
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    this.refreshTimer = null;
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.retryTimer = null;
  }

  /** Feed the FSM with a lifecycle fact and execute its effects. */
  private signal(
    event:
      | { type: "open_started" }
      | { type: "open_failed"; error: Error }
      | { type: "ready" }
      | { type: "transport_disconnect" }
      | { type: "auth_failed" }
      | { type: "fatal"; error: Error }
      | { type: "revoked" }
      | { type: "unpair" },
  ): void {
    const action = transition(this.state, event);
    if (action.next !== this.state) {
      this.state = action.next;
      this.callbacks.onConnectionState(action.next);
    }
    for (const effect of action.effects) {
      if (effect.type === "dispose_connection") {
        this.disposeConnection(effect.reason);
      } else if (effect.type === "schedule_reconnect") {
        // Already handled by scheduleRetry (the only timer owner).
      } else if (effect.type === "begin_token_refresh") {
        // The refresh already started; the timer logic lives in refreshToken.
      }
    }
  }

  /**
   * Close and drop the current connection if any — the single disposal path.
   * Every transition that leaves the connected states funnels through here, so
   * a leaked NATS connection (M10) is structurally impossible.
   */
  private disposeConnection(reason: string): void {
    const connection = this.connection;
    this.connection = null;
    this.handshakePromise = null;
    this.confirmedBridgeInstanceId = "";
    this.rejectDownloadWaiters(reason);
    if (connection) void connection.close().catch(() => undefined);
  }

  private subscribeTransfers(connection: NatsConnection, generation: number): void {
    const prefix = `p.${this.credentials.pairId}.xfer.down.`;
    const subscription = connection.subscribe(`${prefix}>`);
    void (async () => {
      let liveGeneration = true;
      let failure: unknown = new Error("remote_transfer_subscription_ended");
      try {
        for await (const message of subscription) {
          if (this.stopped || generation !== this.generation) {
            liveGeneration = false;
            break;
          }
          const suffix = message.subject.startsWith(prefix)
            ? message.subject.slice(prefix.length)
            : "";
          const parts = suffix.split(".");
          if (parts.length !== 3 || parts[1] !== "chunk") continue;
          const key = `${parts[0]}:${parts[2]}`;
          const waiter = this.downloadWaiters.get(key);
          if (!waiter) continue;
          clearTimeout(waiter.timer);
          this.downloadWaiters.delete(key);
          waiter.resolve(message.data);
        }
      } catch (error) {
        if (!this.stopped && generation === this.generation) {
          failure = error;
        } else {
          liveGeneration = false;
        }
      }
      if (liveGeneration) {
        this.failGeneration(failure, generation);
      }
    })();
  }

  private subscribeEvents(connection: NatsConnection, generation: number): void {
    const subscription = connection.subscribe(`p.${this.credentials.pairId}.evt.>`);
    void (async () => {
      let liveGeneration = true;
      let failure: unknown = new Error("remote_event_subscription_ended");
      try {
        for await (const message of subscription) {
          if (this.stopped || generation !== this.generation) {
            liveGeneration = false;
            break;
          }
          const prefix = `p.${this.credentials.pairId}.evt.`;
          const sessionId = message.subject.startsWith(prefix)
            ? message.subject.slice(prefix.length)
            : "";
          let event: StreamEvent;
          try {
            event = decodeJson<StreamEvent>(message.data);
          } catch (error) {
            // A single malformed event must not kill the whole subscription
            // (L6). Reconcile this subject immediately: if the malformed frame
            // was the run's final event there may be no later idx gap to trigger
            // the normal replay path.
            this.callbacks.onEventDecodeFailure(
              sessionId,
              errorWithContext("remote_event_decode_failed", error),
            );
            continue;
          }
          this.callbacks.onEvent(event, sessionId);
        }
      } catch (error) {
        if (!this.stopped && generation === this.generation) {
          failure = error;
        } else {
          liveGeneration = false;
        }
      }
      // A subscription can fail independently of the NATS status iterator.
      // Restart the whole connection generation so realtime delivery cannot die
      // silently while commands and presence still appear healthy.
      if (liveGeneration && !this.stopped && generation === this.generation) {
        this.failGeneration(failure, generation);
      }
    })();
  }

  private subscribeLiveness(connection: NatsConnection, generation: number): void {
    const subscription = connection.subscribe(`p.${this.credentials.pairId}.presence`);
    void (async () => {
      let liveGeneration = true;
      let failure: unknown = new Error("remote_presence_subscription_ended");
      try {
        for await (const message of subscription) {
          if (this.stopped || generation !== this.generation) {
            liveGeneration = false;
            break;
          }
          let presence: Presence;
          try {
            presence = decodeJson<Presence>(message.data);
          } catch {
            continue;
          }
          if (
            !presence.bridgeInstanceId ||
            presence.bridgeInstanceId !== this.confirmedBridgeInstanceId
          ) {
            // The bridge restarted — the confirmed handshake is stale. Rotate
            // back through the handshake to re-bind, then resync.
            try {
              const confirmation = await this.ensureHandshake(connection);
              if (this.stopped || generation !== this.generation) break;
              this.confirmedBridgeInstanceId = confirmation.bridgeInstanceId;
              this.callbacks.onPresence(confirmation.presence);
              this.callbacks.onFeatures(confirmation.features ?? []);
              this.callbacks.onReconnected();
            } catch (error) {
              this.failGeneration(error, generation);
              return;
            }
          } else {
            this.callbacks.onPresence(presence);
          }
        }
      } catch (error) {
        if (!this.stopped && generation === this.generation) {
          failure = error;
        } else {
          liveGeneration = false;
        }
      }
      if (liveGeneration) {
        this.failGeneration(failure, generation);
      }
    })();
  }

  private subscribeState(connection: NatsConnection, generation: number): void {
    const subscription = connection.subscribe(`p.${this.credentials.pairId}.state.>`);
    void (async () => {
      let liveGeneration = true;
      let failure: unknown = new Error("remote_state_subscription_ended");
      try {
        for await (const message of subscription) {
          if (this.stopped || generation !== this.generation) {
            liveGeneration = false;
            break;
          }
          const suffix = message.subject.slice(`p.${this.credentials.pairId}.state.`.length);
          if (suffix === "sessions") {
            const data = decodeJson<{ sessions?: PresenceSession[] }>(message.data);
            this.callbacks.onSessions(data.sessions ?? []);
          } else if (suffix === "workspaces") {
            const data = decodeJson<{ workspaces?: RemoteWorkspace[] }>(message.data);
            this.callbacks.onWorkspaces(data.workspaces ?? []);
          }
        }
      } catch (error) {
        if (!this.stopped && generation === this.generation) {
          failure = error;
        } else {
          liveGeneration = false;
        }
      }
      if (liveGeneration) {
        this.failGeneration(failure, generation);
      }
    })();
  }

  /**
   * The NATS status loop is the transport-truth feeder. A disconnect while
   * ready enters reconnecting WITHOUT arming our own timer — NATS reconnects
   * this same connection and emits a `reconnect` status, which we treat as the
   * re-bound handshake. The internal budget is finite (maxReconnectAttempts ≈
   * 10 → ~30s): once it is spent, this for-await loop simply ENDS — no status,
   * no timer, and the app would sit in "reconnecting" forever on a dead
   * connection. A normal loop exit therefore falls back to `open_failed`, which
   * arms the single-owner backoff timer so the FSM keeps retrying.
   */
  private watchStatus(connection: NatsConnection, generation: number): void {
    void (async () => {
      let exitedNaturally = true;
      try {
        for await (const status of connection.status()) {
          if (this.stopped || generation !== this.generation) {
            exitedNaturally = false;
            break;
          }
          if (status.type === "disconnect") {
            this.signal({ type: "transport_disconnect" });
          } else if (status.type === "reconnect") {
            if (this.connection !== connection) continue;
            try {
              const confirmation = await this.ensureHandshake(connection);
              if (this.stopped || generation !== this.generation) {
                exitedNaturally = false;
                break;
              }
              this.confirmedBridgeInstanceId = confirmation.bridgeInstanceId;
              this.callbacks.onPresence(confirmation.presence);
              this.callbacks.onFeatures(confirmation.features ?? []);
              this.signal({ type: "ready" });
              this.callbacks.onReconnected();
            } catch (error) {
              this.failGeneration(error, generation);
              exitedNaturally = false;
              return;
            }
          } else if (status.type === "error") {
            const code = String(status.data);
            console.warn("[remote] NATS asynchronous error", {
              code,
              permissionContext: status.permissionContext,
            });
            if (code === ErrorCode.AuthenticationExpired || code === ErrorCode.AccountExpired) {
              if (this.state !== "refreshing") void this.refreshToken();
            } else if (
              code === ErrorCode.PermissionsViolation ||
              code === ErrorCode.AuthorizationViolation
            ) {
              this.handleFailure(new Error(`remote_service_misconfigured: ${code}`));
              return;
            } else if (code === ErrorCode.ProtocolError) {
              this.failGeneration(new Error(`nats_protocol_error: ${code}`), generation);
              return;
            }
          }
        }
      } catch (error) {
        if (!this.stopped) this.callbacks.onError(asError(error));
      }
      // The status iterator ended (or threw) while this is still the live
      // generation: NATS's internal reconnect budget was spent and the
      // connection is permanently closed. Treat it as a dead attempt so the
      // backoff timer (single owner) keeps the app reconnecting — without this,
      // a >30s outage wedges the UI in "reconnecting" until a JWT refresh
      // happens to fire.
      if (exitedNaturally && !this.stopped && generation === this.generation) {
        this.failGeneration(new Error("nats_connection_exhausted"), generation);
      }
    })();
  }

  private scheduleRefresh(): void {
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    const expiry = jwtExpiry(this.credentials.userJwt);
    if (expiry === null) {
      // Malformed JWT — mirror the desktop's hard reject: surface the error
      // and drop the connection instead of entering a 5s refresh loop.
      this.callbacks.onError(new Error("invalid_jwt"));
      void this.close();
      return;
    }
    const delay = Math.max(5_000, expiry * 1000 - Date.now() - 60_000);
    this.refreshTimer = setTimeout(() => {
      this.refreshTimer = null;
      void this.refreshToken();
    }, delay);
  }

  async request<T>(
    command: RemoteCommand,
    sessionId = command.sessionId ?? "list",
    timeoutMs = 10_000,
  ): Promise<RpcResponse<T>> {
    const connection = this.connection;
    if (!connection) throw new Error("not_connected");
    return this.requestWithConnection(connection, command, sessionId, timeoutMs);
  }

  /** Recover a stale request path only for transport failures. Business errors
   * must surface immediately and must never enter the transfer retry loop. */
  async recoverAfterTransientRequest(error: unknown): Promise<boolean> {
    if (!isTransientNatsRequestError(error)) return false;
    // The retry itself remains the source of truth. Recovery is best-effort:
    // if reconnecting fails, let the bounded retry report its own transport
    // error instead of replacing it with an internal recovery exception.
    await this.recoverNow("request-failure").catch(() => undefined);
    return true;
  }

  /**
   * Retry a control-plane command without changing its identity. The desktop
   * deduplicates command ids, so a reply lost after successful execution is
   * replayed instead of executing the operation twice.
   */
  async requestRetry<T>(
    command: RemoteCommand,
    sessionId = command.sessionId ?? "list",
  ): Promise<RpcResponse<T>> {
    const stableCommand = { ...command, id: command.id ?? randomId("cmd") };
    let lastError: unknown;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      try {
        return await this.request<T>(stableCommand, sessionId);
      } catch (error) {
        lastError = error;
        if (!isTransientNatsRequestError(error)) throw error;
        if (attempt < 2) {
          await new Promise(resolve => setTimeout(resolve, 250 * 2 ** attempt));
        }
      }
    }
    throw lastError;
  }

  async uploadChunk(transferId: string, index: number, bytes: Uint8Array): Promise<void> {
    const connection = this.connection;
    if (!connection) throw new Error("not_connected");
    const message = await connection.request(
      `p.${this.credentials.pairId}.xfer.up.${transferId}.chunk.${index}`,
      bytes,
      { timeout: 15_000 },
    );
    const response = decodeJson<RpcResponse>(message.data);
    if (!response.success) throw new Error(response.error ?? "upload_chunk_failed");
  }

  async downloadChunk(transferId: string, index: number): Promise<Uint8Array> {
    const connection = this.connection;
    if (!connection) throw new Error("not_connected");
    const key = `${transferId}:${index}`;
    const pending = new Promise<Uint8Array>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.downloadWaiters.delete(key);
        reject(new Error("download_chunk_timeout"));
      }, 15_000);
      this.downloadWaiters.set(key, { resolve, reject, timer });
    });
    // The binary chunk may time out before the pull ACK settles. Attach a
    // handler immediately so React Native never reports that early rejection
    // as unhandled; the original promise is still awaited and rethrows below.
    void pending.catch(() => undefined);
    try {
      const message = await connection.request(
        `p.${this.credentials.pairId}.xfer.up.${transferId}.pull.${index}`,
        new Uint8Array(),
        { timeout: 15_000 },
      );
      const response = decodeJson<RpcResponse>(message.data);
      if (!response.success) throw new Error(response.error ?? "download_chunk_failed");
      return await pending;
    } catch (error) {
      const waiter = this.downloadWaiters.get(key);
      if (waiter) clearTimeout(waiter.timer);
      this.downloadWaiters.delete(key);
      throw error;
    }
  }

  private async requestWithConnection<T>(
    connection: NatsConnection,
    command: RemoteCommand,
    sessionId = command.sessionId ?? "list",
    timeoutMs = 10_000,
  ): Promise<RpcResponse<T>> {
    const payload = { ...command, id: command.id ?? randomId("cmd") };
    const message = await connection.request(
      `p.${this.credentials.pairId}.cmd.${sessionId || "new"}`,
      encoder.encode(JSON.stringify(payload)),
      { timeout: timeoutMs },
    );
    const response = decodeJson<RpcResponse<T>>(message.data);
    if (!response.success) {
      const error = new RemoteResponseError(response.error ?? "command_failed");
      if (classifyError(error) === "authTerminal") this.signal({ type: "revoked" });
      throw error;
    }
    return response;
  }

  private async performHandshake(connection: NatsConnection): Promise<HandshakeConfirmation> {
    const keyPair = fromSeed(encoder.encode(this.credentials.seed));
    const clientPublicKey = keyPair.getPublicKey();
    const clientNonce = randomId("challenge");
    const challengeResponse = await this.requestWithConnection<HandshakeChallenge>(
      connection,
      {
        type: "pair_handshake",
        protocolVersion: HANDSHAKE_PROTOCOL_VERSION,
        pairId: this.credentials.pairId,
        deviceId: this.credentials.deviceId,
        clientPublicKey,
        clientNonce,
        expectedDesktopId: this.credentials.expectedDesktopId,
        expectedDesktopPublicKey: this.credentials.expectedDesktopPublicKey,
      },
      "handshake",
    );
    const challenge = challengeResponse.data;
    if (
      !verifyDesktopChallenge(challenge, {
        pairId: this.credentials.pairId,
        desktopId: this.credentials.expectedDesktopId,
        desktopPublicKey: this.credentials.expectedDesktopPublicKey,
        deviceId: this.credentials.deviceId,
        clientPublicKey,
        clientNonce,
      })
    ) {
      throw new Error("pairing_signature_invalid");
    }
    const transcript = handshakeTranscript(challenge);
    const clientSignature = encodeBase64Url(keyPair.sign(encoder.encode(transcript)));
    const confirmationResponse = await this.requestWithConnection<HandshakeConfirmation>(
      connection,
      {
        type: "pair_handshake_confirm",
        deviceId: this.credentials.deviceId,
        desktopNonce: challenge.desktopNonce,
        clientSignature,
      },
      "handshake",
    );
    const confirmation = confirmationResponse.data;
    if (
      !confirmation.confirmed ||
      confirmation.pairId !== this.credentials.pairId ||
      confirmation.desktopId !== this.credentials.expectedDesktopId ||
      confirmation.bridgeInstanceId !== challenge.bridgeInstanceId ||
      confirmation.deviceId !== this.credentials.deviceId ||
      confirmation.desktopNonce !== challenge.desktopNonce ||
      confirmation.presence.bridgeInstanceId !== challenge.bridgeInstanceId
    ) {
      throw new Error("pairing_confirmation_mismatch");
    }
    this.confirmedBridgeInstanceId = confirmation.bridgeInstanceId;
    return confirmation;
  }

  private ensureHandshake(connection: NatsConnection): Promise<HandshakeConfirmation> {
    if (this.handshakePromise) return this.handshakePromise;
    const pending = this.performHandshake(connection);
    this.handshakePromise = pending;
    const clear = () => {
      if (this.handshakePromise === pending) this.handshakePromise = null;
    };
    void pending.then(clear, clear);
    return pending;
  }
}

class RemoteResponseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RemoteResponseError";
  }
}

export function isTransientNatsRequestError(error: unknown): boolean {
  if (error instanceof RemoteResponseError) return false;
  if (error instanceof Error && error.message === "not_connected") return true;
  const code =
    typeof error === "object" && error !== null && "code" in error
      ? (error as { code?: unknown }).code
      : undefined;
  if (typeof code !== "string") return false;
  return new Set<string>([
    ErrorCode.Timeout,
    ErrorCode.NoResponders,
    ErrorCode.ConnectionClosed,
    ErrorCode.Disconnect,
    ErrorCode.RequestError,
  ]).has(code);
}

function asError(value: unknown): Error {
  return errorWithContext("remote_error", value);
}

function errorWithContext(context: string, value: unknown): Error {
  const record =
    typeof value === "object" && value !== null ? (value as Record<string, unknown>) : null;
  const rawMessage = value instanceof Error ? value.message : value;
  const message = typeof rawMessage === "string" ? rawMessage.trim() : "";
  const code = typeof record?.code === "string" ? record.code.trim() : "";
  const name = typeof record?.name === "string" ? record.name.trim() : "";
  const detail = [code, message, name && name !== "Error" ? name : ""].find(Boolean) ?? "unknown";
  const error = new Error(`${context}: ${detail}`);
  if (value instanceof Error) error.cause = value;
  return error;
}

function safeServerLabel(value: string): string {
  try {
    const url = new URL(value);
    return `${url.protocol}//${url.host}${url.pathname}`;
  } catch {
    return "invalid server URL";
  }
}
