import { ConnectionGeneration } from "./connectionGeneration";
import type { Msg, NatsConnection, Subscription } from "@nats-io/nats-core";
import { wsconnect, jwtAuthenticator } from "@nats-io/nats-core";
import { classifyNatsError } from "./natsErrors";
import { SecureChannel, SecureHandshake, replyContext, type SecureIdentity } from "./secureChannel";
import { ensureFreshCredentials, refreshCredentials } from "./pairing";
import { jwtExpiry, randomId, encodeBase64Url, decodeBase64Url } from "./codec";
import { backoffDelayMs, classifyError, transition, type ConnectionState } from "./connectionState";
import { decodeRemoteJson, decodeRemoteJsonAsync } from "./remoteJson";
import type {
  SnapshotVersion,
  Presence,
  PresenceSession,
  RemoteCommand,
  RemoteCredentials,
  RemoteWorkspace,
  RpcResponse,
  StreamEvent,
} from "./types";

const encoder = new TextEncoder();
const FOREGROUND_PROBE_TIMEOUT_MS = 4_000;
const SYSTEM_FAILURE_WINDOW_MS = 10 * 60_000;
const HEALTHY_RESET_MS = 60_000;
const MAX_SYSTEM_RECOVERIES = 3;
const FAILURE_LOG_WINDOW_MS = 24 * 60 * 60_000;
const MAX_FAILURE_LOGS_PER_CATEGORY = 16;
export type RecoveryReason =
  "foreground" | "network-restored" | "network-changed" | "request-failure" | "presence-stale";

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
  onCredentials(credentials: RemoteCredentials): Promise<void>;
  onEvent(event: StreamEvent, sessionId: string): void;
  onEventDecodeFailure(sessionId: string, error: Error): void;
  onCatalogEpoch?(epoch: string | undefined): void;
  onPresence(presence: Presence): void;
  onSessions(sessions: PresenceSession[], version?: SnapshotVersion): void;
  onWorkspaces(workspaces: RemoteWorkspace[], version?: SnapshotVersion): void;
  onFeatures(features: string[]): void;
  onConnectionState(state: ConnectionState): void;
  onReconnected(): void;
  onError(error: Error): void;
}

function failureSupportCode(category: string, detail = ""): string {
  if (category === "network") return "NW001";
  if (category === "credential_revoked") return "PA001";
  if (category === "credential_expired") return "AU002";
  if (category === "service_authorization") return "AU001";
  if (category === "protocol" || detail.includes("protocol")) return "PT001";
  if (category === "generation_unhealthy") return "RT001";
  if (category === "local") return "LC001";
  return "LC999";
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
  private visibleSessionId = "";
  private eventSubscriptions = new Map<NatsConnection, { generation: number; selective: boolean; revision: number; subscription?: Subscription }>();

  setVisibleSession(sessionId: string): void {
    const next = /^[A-Za-z0-9_-]+$/.test(sessionId) ? sessionId : "";
    if (this.visibleSessionId === next) return;
    this.visibleSessionId = next;
    for (const [connection, record] of this.eventSubscriptions) {
      if (record.selective) this.installEventSubscription(connection);
    }
  }

  private everReady = false;
  private recoveryDeadline = 0;
  private deadlineTimer: ReturnType<typeof setTimeout> | null = null;
  private desktopAvailable = false;
  private presenceTimer: ReturnType<typeof setTimeout> | null = null;

  private endUnavailable(error: Error): void {
    if (this.stopped || this.isTerminal()) return;
    this.cancelAttempt();
    this.clearTimers();
    this.clearDeadline();
    if (this.presenceTimer) clearTimeout(this.presenceTimer);
    this.signal({ type: "fatal", error });
    this.callbacks.onError(error);
  }

  private clearDeadline(): void {
    if (this.deadlineTimer) clearTimeout(this.deadlineTimer);
    this.deadlineTimer = null;
    this.recoveryDeadline = 0;
  }

  private beginDeadline(): boolean {
    // OS suspension is not an active recovery attempt. Network notifications
    // and open() can still arrive in the background; none may start a budget.
    if (!this.appActive) return true;
    if (!this.recoveryDeadline) {
      this.recoveryDeadline = Date.now() + (this.everReady ? 180_000 : 20_000);
      this.deadlineTimer = setTimeout(
        () => this.endUnavailable(new Error(this.everReady ? "recovery_timeout" : "connection_timeout")),
        this.recoveryDeadline - Date.now(),
      );
    }
    if (Date.now() >= this.recoveryDeadline) {
      this.endUnavailable(new Error(this.everReady ? "recovery_timeout" : "connection_timeout"));
      return false;
    }
    return true;
  }

  private assertBusinessReady(): void {
    if (this.stopped || this.state !== "ready" || !this.desktopAvailable)
      throw new Error("communication_frozen");
  }

  private receivePresence(presence: Presence): void {
    this.desktopAvailable = presence.online && presence.agentAvailable !== false;
    if (this.presenceTimer) clearTimeout(this.presenceTimer);
    if (!presence.disconnected) {
      this.presenceTimer = setTimeout(() => {
        this.desktopAvailable = false;
        void this.recoverNow("network-changed");
      }, 15_000);
    }
    if (this.desktopAvailable && this.state === "ready") {
      if (this.recoveryDeadline && !this.beginDeadline()) return;
      this.everReady = true;
      this.clearDeadline();
    }
    this.callbacks.onPresence(presence);
  }
  private connection: NatsConnection | null = null;
  private credentials: RemoteCredentials;
  private refreshTimer: ReturnType<typeof setTimeout> | null = null;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private healthyTimer: ReturnType<typeof setTimeout> | null = null;
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
  private activeGeneration: ConnectionGeneration | null = null;
  private candidateGeneration: ConnectionGeneration | null = null;
  private candidateConnection: NatsConnection | null = null;
  private attemptController: AbortController | null = null;
  private openPromise: Promise<void> | null = null;
  private stopped = false;
  private appActive = true;
  private recoveryPromise: Promise<void> | null = null;
  private failedGeneration: number | null = null;
  private state: ConnectionState = "unpaired";
  private negotiatedFeatures = new Set<string>();
  private confirmedBridgeInstanceId = "";
  private readonly handshakes = new WeakMap<NatsConnection, Promise<HandshakeConfirmation>>();
  private readonly secureChannels = new WeakMap<NatsConnection, SecureChannel>();
  /** Guards against overlapping token refresh (timer + auth failure racing). */
  private refreshInFlight = false;
  private systemFailures: number[] = [];
  private failureEpisode: {
    category: string;
    startedAt: number;
    attempts: number;
    reports: number;
    lastError: string;
    nextRetryAt: number | null;
  } | null = null;
  private readonly failureLogQuota = new Map<
    string,
    { windowStartedAt: number; emitted: number }
  >();
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

  private isTerminal(): boolean {
    return this.state === "failed" || this.state === "revoked";
  }

  /** Keep a category bounded even when a broken connection alternates error
   * shapes and would otherwise create a fresh episode for every retry. */
  private permitFailureLog(category: string): boolean {
    const now = Date.now();
    const quota = this.failureLogQuota.get(category);
    if (!quota || now - quota.windowStartedAt >= FAILURE_LOG_WINDOW_MS) {
      this.failureLogQuota.set(category, { windowStartedAt: now, emitted: 1 });
      return true;
    }
    if (quota.emitted >= MAX_FAILURE_LOGS_PER_CATEGORY) return false;
    quota.emitted += 1;
    return true;
  }

  private recordFailure(category: string, error: unknown, nextRetryAt: number | null = null): void {
    const message = error instanceof Error ? error.message : String(error);
    if (!this.failureEpisode || this.failureEpisode.category !== category) {
      if (this.failureEpisode) this.finishFailureEpisode("category_changed");
      this.failureEpisode = {
        category,
        startedAt: Date.now(),
        attempts: 1,
        reports: 0,
        lastError: message,
        nextRetryAt,
      };
      if (this.permitFailureLog(category)) {
        this.failureEpisode.reports = 1;
        console.warn("[remote] failure episode started", {
          ...this.failureEpisode,
          supportCode: failureSupportCode(category, message),
        });
      }
      return;
    }
    const episode = this.failureEpisode;
    episode.attempts += 1;
    episode.lastError = message;
    episode.nextRetryAt = nextRetryAt;
    // Start + at most 14 power-of-two progress lines + recovery/terminal keeps
    // a 24-hour outage bounded to 16 lines per category.
    if (
      episode.reports < 15 &&
      (episode.attempts & (episode.attempts - 1)) === 0 &&
      this.permitFailureLog(category)
    ) {
      episode.reports += 1;
      console.warn("[remote] failure episode retry", {
        ...episode,
        supportCode: failureSupportCode(category, message),
      });
    }
  }

  private finishFailureEpisode(result: "recovered" | "terminal" | "category_changed"): void {
    const episode = this.failureEpisode;
    if (!episode) return;
    if (this.permitFailureLog(episode.category)) {
      console.warn("[remote] failure episode finished", {
        ...episode,
        result,
        supportCode: failureSupportCode(episode.category, episode.lastError),
        durationMs: Date.now() - episode.startedAt,
      });
    }
    this.failureEpisode = null;
  }

  /**
   * Establish (or re-establish) the live connection. The attempt runs in the
   * background: on a transport failure this arms the backoff timer and returns,
   * so a slow attempt never blocks its caller (H4's "close() then reconnect
   * 1s later" storm is structurally impossible — close never arms a timer, and
   * the retry timer is owned by exactly one place, here).
   */
  open(): Promise<void> {
    if (this.stopped || this.isTerminal() || !this.beginDeadline()) return Promise.resolve();
    if (this.openPromise) return this.openPromise;
    const pending = this.openAttempt().finally(() => {
      if (this.openPromise === pending) this.openPromise = null;
    });
    this.openPromise = pending;
    return pending;
  }

  private async openAttempt(): Promise<void> {
    if (this.stopped || this.isTerminal()) return;
    if (!this.appActive) return;
    this.signal({ type: "open_started" });
    const generation = ++this.generation;
    const controller = new AbortController();
    this.attemptController = controller;
    const current = () =>
      !controller.signal.aborted && !this.stopped && generation === this.generation;
    try {
      await withTimeout((async () => {
        const previous = this.credentials;
        const fresh = await ensureFreshCredentials(previous, controller.signal);
        if (!current()) return;
        if (fresh !== previous) {
          await this.callbacks.onCredentials(fresh);
          if (!current()) return;
        }
        this.credentials = fresh;
        await this.connectSocket(generation);
      })(), Math.min(20_000, this.recoveryDeadline - Date.now()));
      if (current()) this.scheduleRefresh();
    } catch (error) {
      if (current()) {
        // A failed replacement does not prove the serving socket failed. Verify
        // both directions with an encrypted RPC before disabling its UI or
        // starting another replacement. Never mask identity/auth failures.
        if (classifyError(error) === "transport" && await this.restoreServingConnection(generation)) return;
        if (!current()) return;
        this.cancelAttempt();
        this.openPromise = null;
        this.handleFailure(error);
      }
    } finally {
      if (this.attemptController === controller) this.attemptController = null;
    }
  }

  private async restoreServingConnection(generation: number): Promise<boolean> {
    const connection = this.connection;
    const owner = this.activeGeneration;
    if (!connection || !owner?.live || this.failedGeneration === owner.id || connection.isClosed()) return false;
    try {
      const { data: presence } = await this.requestWithConnection<Presence>(
        connection, { type: "get_presence" }, "list", FOREGROUND_PROBE_TIMEOUT_MS,
      );
      if (this.stopped || this.isTerminal() || !this.appActive ||
          generation !== this.generation || connection !== this.connection || !owner.live ||
          this.failedGeneration === owner.id || !presence.online ||
          presence.pairId !== this.credentials.pairId || presence.bridgeInstanceId !== this.confirmedBridgeInstanceId) return false;
      if (this.retryTimer) clearTimeout(this.retryTimer);
      this.retryTimer = null;
      this.retryAttempt = 0;
      if (this.recoveryDeadline && !this.beginDeadline()) return false;
      this.receivePresence(presence);
      if (this.stopped || this.isTerminal()) return false;
      this.signal({ type: "ready" });
      this.scheduleRefresh();
      this.finishFailureEpisode("recovered");
      this.callbacks.onReconnected();
      return true;
    } catch {
      // The old peer/channel really is gone: retain the bounded retry path.
      return false;
    }
  }

  private cancelAttempt(): void {
    this.generation += 1;
    this.attemptController?.abort();
    this.attemptController = null;
    this.openPromise = null;
    this.candidateGeneration?.retire();
    this.candidateGeneration = null;
    const candidate = this.candidateConnection;
    this.candidateConnection = null;
    if (candidate) {
      this.secureChannels.get(candidate)?.destroy();
      this.secureChannels.delete(candidate);
      void candidate.close().catch(() => undefined);
    }
  }

  private isLiveGeneration(generation: number): boolean {
    return (
      !this.stopped &&
      ((this.activeGeneration?.id === generation && this.activeGeneration.live) ||
        (this.candidateGeneration?.id === generation && this.candidateGeneration.live))
    );
  }

  /** Full teardown — never arms a retry, never broadcasts a phase. Idempotent. */
  async close(reason: "UserInitiated" | "Unpair" = "UserInitiated"): Promise<void> {
    if (this.stopped) return;
    this.stopped = true;
    this.clearDeadline();
    if (this.presenceTimer) clearTimeout(this.presenceTimer);
    this.cancelAttempt();
    this.clearTimers();
    this.disposeConnection(reason === "UserInitiated" ? "close" : "unpair");
    this.rejectDownloadWaiters("closed");
    this.state = reason === "UserInitiated" ? "stopped" : "unpaired";
  }

  /**
   * Ordinary WebSockets are not a background execution mechanism on either
   * mobile OS. Close deliberately before JavaScript is suspended so foreground
   * recovery starts from a known state instead of inheriting a half-open WSS.
   * AppState owns this flag independently of network probes or recovery requests.
   */
  setAppActive(active: boolean): void {
    if (this.stopped || active === this.appActive) return;
    this.appActive = active;
    if (active) {
      // Resume with a fresh bounded window, even if reachability is still
      // offline. Terminal failures and explicit close remain terminal.
      if (!this.isTerminal()) this.beginDeadline();
      return;
    }
    this.clearDeadline();
    if (this.presenceTimer) clearTimeout(this.presenceTimer);
    this.presenceTimer = null;
    this.recoveryPromise = null;
    this.cancelAttempt();
    this.clearTimers();
    this.signal({ type: "transport_disconnect" });
    this.disposeConnection("background");
  }

  /** Validate after foregrounding, or immediately rebuild after a path change.
   * Native reachability is only a hint: actual connection failures own the
   * bounded retry/backoff, even when the OS keeps reporting offline. */
  recoverNow(reason: RecoveryReason): Promise<void> {
    if (this.stopped || this.isTerminal()) return Promise.resolve();
    if (this.recoveryDeadline && !this.beginDeadline()) return Promise.resolve();
    if (!this.appActive) return Promise.resolve();
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
      // Missing authenticated presence is not a broker-connectivity problem.
      // After a desktop restart its traffic keys are gone, while NATS still
      // happily answers PING. That path must establish a new secure channel.
      reason !== "presence-stale" &&
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
          this.appActive &&
          generation === this.generation &&
          connection === this.connection
        ) {
          // A broker PONG does not prove the desktop still has our traffic
          // keys (it may have restarted while the phone was asleep). Validate
          // the authenticated command path before reusing it on foreground.
          if ((this.state === "ready" && reason !== "foreground") ||
              await this.restoreServingConnection(generation)) return;
        }
      } catch {
        // Rebuild below without waiting for NATS's ping budget to expire.
      }
    }
    if (this.stopped || !this.appActive || generation !== this.generation)
      return;
    // Keep the old generation as the serving fallback while the replacement
    // completes its handshake, subscriptions, and flush. `connectSocket()`
    // atomically publishes the new connection and only then closes this one.
    this.clearTimers();
    this.signal({ type: "transport_disconnect" });
    this.retryAttempt = 0;
    await this.open();
  }

  /** Rotate the JWT in place and resume the connection (M1's refreshable class). */
  private async refreshToken(): Promise<void> {
    if (
      this.stopped ||
      this.refreshInFlight ||
      this.openPromise ||
      this.isTerminal() ||
      !this.appActive
    )
      return;
    this.refreshInFlight = true;
    const generation = this.generation;
    const controller = new AbortController();
    this.attemptController = controller;
    try {
      this.signal({ type: "auth_failed" }); // moves to refreshing
      const fresh = await refreshCredentials(this.credentials, controller.signal);
      if (this.stopped || controller.signal.aborted) return;
      this.credentials = fresh;
      await this.callbacks.onCredentials(fresh);
      if (this.stopped || controller.signal.aborted) return;
      // Re-open with the fresh token while retaining the old generation until
      // the replacement passes its readiness barrier. The FSM is already in
      // refreshing, so this does not expose a second ready connection.
      await this.open();
    } catch (error) {
      if (this.stopped || controller.signal.aborted) return;
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
      if (classifyError(error) === "transport" && await this.restoreServingConnection(generation)) return;
      if (this.stopped || controller.signal.aborted) return;
      this.callbacks.onError(error instanceof Error ? error : new Error(String(error)));
      this.scheduleRetry();
    } finally {
      if (this.attemptController === controller) this.attemptController = null;
      this.refreshInFlight = false;
    }
  }

  private async connectSocket(generation: number): Promise<void> {
    const seed = encoder.encode(this.credentials.seed);
    let connection: NatsConnection;
    try {
      connection = await wsconnect({
        servers: this.credentials.natsWsUrl,
        timeout: 10_000,
        maxReconnectAttempts: 0,
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
    const candidate = new ConnectionGeneration(generation);
    this.candidateGeneration = candidate;
    this.candidateConnection = connection;
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
      // Install every critical subscription and flush their SUB frames before
      // publishing this generation as ready. Until this barrier succeeds the
      // previous generation remains the sole externally-ready connection.
      this.subscribeEvents(connection, generation, confirmation.features?.includes("selective_events_v1") === true);
      this.subscribeLiveness(connection, generation);
      this.subscribeState(connection, generation);
      this.subscribeTransfers(connection, generation);
      await withTimeout(connection.flush(), 10_000);
      await this.activateSecureChannel(connection);
      candidate.check();
      if (this.stopped || this.isTerminal() || generation !== this.generation) {
        await connection.close().catch(() => undefined);
        return;
      }
      // Dispose any prior connection ONLY after the new one is proven — M10's
      // ghost connection: the old path nulled `this.connection` before the
      // refresh, leaking it on failure.
      const previous = this.connection;
      const previousGeneration = this.activeGeneration;
      this.activeGeneration = candidate;
      this.candidateGeneration = null;
      this.candidateConnection = null;
      this.connection = connection;
      previousGeneration?.retire();
      this.confirmedBridgeInstanceId = confirmation.bridgeInstanceId;
      this.retryAttempt = 0;
      this.authRetryCount = 0;
      this.failedGeneration = null;
      this.watchStatus(connection, generation);
      if (previous && previous !== connection) {
        this.eventSubscriptions.delete(previous);
        this.secureChannels.get(previous)?.destroy();
        this.secureChannels.delete(previous);
        void previous.close().catch(() => undefined);
      }
      this.callbacks.onCatalogEpoch?.(confirmation.presence.catalogEpoch);
      this.receivePresence(confirmation.presence);
      this.negotiatedFeatures = new Set(confirmation.features ?? []);
      this.callbacks.onFeatures(confirmation.features ?? []);
      candidate.activate();
      if (this.stopped || this.connection !== connection) return;
      this.signal({ type: "ready" });
      this.finishFailureEpisode("recovered");
      if (this.healthyTimer) clearTimeout(this.healthyTimer);
      this.healthyTimer = setTimeout(() => {
        this.healthyTimer = null;
        if (this.state === "ready" && generation === this.generation) this.systemFailures = [];
      }, HEALTHY_RESET_MS);
      this.callbacks.onReconnected();
    } catch (error) {
      // Subscriptions are deliberately created before the readiness flush.
      // Closing a half-built connection naturally ends those iterators; mark
      // this generation failed first so they cannot recast a transport/readiness
      // failure as four independent critical subscription failures.
      if (!this.stopped && generation === this.generation) {
        this.failedGeneration = generation;
      }
      candidate.retire();
      this.eventSubscriptions.delete(connection);
      if (this.candidateGeneration === candidate) {
        this.candidateGeneration = null;
        this.candidateConnection = null;
      }
      this.secureChannels.get(connection)?.destroy();
      this.secureChannels.delete(connection);
      await connection.close().catch(() => undefined);
      throw error;
    }
  }

  private handleFailure(error: unknown): void {
    if (this.stopped || this.isTerminal()) return;
    if (!this.everReady && classifyError(error) !== "authTerminal") {
      this.endUnavailable(error instanceof Error ? error : new Error(String(error)));
      return;
    }
    const kind = classifyError(error);
    if (kind === "authTerminal") {
      // The device was revoked (M1) — stop every network action and tell the
      // UI to guide the user to re-pair.
      this.cancelAttempt();
      this.failedGeneration = null;
      this.clearTimers();
      this.signal({ type: "revoked" });
      this.recordFailure("credential_revoked", error);
      this.finishFailureEpisode("terminal");
      this.callbacks.onError(error instanceof Error ? error : new Error(String(error)));
      return;
    }
    if (kind === "fatal") {
      this.cancelAttempt();
      this.failedGeneration = null;
      this.clearTimers();
      const failure = error instanceof Error ? error : new Error(String(error));
      this.signal({ type: "fatal", error: failure });
      this.recordFailure(
        failure.message.includes("generation_unhealthy")
          ? "generation_unhealthy"
          : "service_authorization",
        failure,
      );
      this.finishFailureEpisode("terminal");
      this.callbacks.onError(failure);
      return;
    }
    if (kind === "auth") {
      // A timer-triggered refresh may overlap an error emitted by the old
      // connection. Do not promote it to terminal, but leave a bounded
      // backoff retry in case the replacement connection rejects too.
      if (this.refreshInFlight) {
        this.scheduleRetry();
        return;
      }
      // Refreshable — rotate the JWT ONCE, then back off. A handshake that
      // keeps failing after a successful refresh must not spin at one full
      // RTT per cycle (no backoff): the second consecutive auth failure takes
      // the shared retry path instead.
      if ((jwtExpiry(this.credentials.userJwt) ?? 0) * 1000 <= Date.now()) this.authRetryCount = 0;
      this.authRetryCount += 1;
      if (this.authRetryCount === 1 && !this.refreshInFlight) {
        void this.refreshToken();
        return;
      }
      // A freshly issued JWT that is rejected by NATS proves this is not token
      // expiry. Stop the loop as a service/account authorization failure.
      const failure = new Error(
        `remote_service_misconfigured: ${error instanceof Error ? error.message : String(error)}`,
      );
      this.cancelAttempt();
      this.failedGeneration = null;
      this.clearTimers();
      this.signal({ type: "fatal", error: failure });
      this.recordFailure("service_authorization", failure);
      this.finishFailureEpisode("terminal");
      this.callbacks.onError(failure);
      return;
    }
    this.callbacks.onError(error instanceof Error ? error : new Error(String(error)));
    this.scheduleRetry();
  }

  /** A live generation may lose several subscriptions at once. Reconnect it
   * exactly once so their terminal callbacks cannot reset the retry timer into
   * a reconnect storm. */
  private failGeneration(error: unknown, generation: number): void {
    if (this.candidateGeneration?.id === generation) {
      this.candidateGeneration.fail(error);
      void this.candidateConnection?.close().catch(() => undefined);
      return;
    }
    if (!this.isLiveGeneration(generation) || this.failedGeneration === generation) {
      return;
    }
    this.failedGeneration = generation;
    if (this.openPromise) {
      this.signal({ type: "transport_disconnect" });
      return;
    }
    const message = error instanceof Error ? error.message : String(error);
    if (message.includes("subscription_ended") || message.includes("protocol_error")) {
      const cutoff = Date.now() - SYSTEM_FAILURE_WINDOW_MS;
      this.systemFailures = this.systemFailures.filter((timestamp) => timestamp >= cutoff);
      this.systemFailures.push(Date.now());
      this.recordFailure("generation_unhealthy", error);
      if (this.systemFailures.length > MAX_SYSTEM_RECOVERIES) {
        this.handleFailure(
          new Error(
            `generation_unhealthy:${message.includes("protocol_error") ? "protocol" : "subscription"}`,
          ),
        );
        return;
      }
    }
    this.handleFailure(error);
  }

  private scheduleRetry(): void {
    if (this.stopped || this.isTerminal()) return;
    if (!this.everReady) {
      this.endUnavailable(new Error("connection_failed"));
      return;
    }
    if (!this.beginDeadline()) return;
    this.signal({ type: "open_failed", error: new Error("transport_failed") });
    if (this.stopped || this.isTerminal() || !this.appActive) return;
    if (this.retryTimer) clearTimeout(this.retryTimer);
    const delay = backoffDelayMs(this.retryAttempt);
    this.recordFailure("network", new Error("transport_failed"), Date.now() + delay);
    this.retryAttempt += 1;
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      if (this.stopped || this.isTerminal() || !this.appActive) return;
      void this.open();
    }, delay);
  }

  private clearTimers(): void {
    if (this.refreshTimer) clearTimeout(this.refreshTimer);
    this.refreshTimer = null;
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.retryTimer = null;
    if (this.healthyTimer) clearTimeout(this.healthyTimer);
    this.healthyTimer = null;
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
    if (event.type === "ready") {
      if (this.stopped || this.isTerminal()) return;
      if (this.recoveryDeadline && !this.beginDeadline()) return;
      if (this.desktopAvailable) {
        this.everReady = true;
        this.clearDeadline();
      }
    } else if (event.type === "transport_disconnect" || event.type === "auth_failed") {
      if (!this.beginDeadline()) return;
    } else if (event.type === "fatal" || event.type === "revoked") {
      this.clearDeadline();
    }
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
    this.activeGeneration?.retire();
    this.activeGeneration = null;
    this.confirmedBridgeInstanceId = "";
    this.rejectDownloadWaiters(reason);
    if (connection) {
      this.eventSubscriptions.delete(connection);
      this.secureChannels.get(connection)?.destroy();
      this.secureChannels.delete(connection);
      void connection.close().catch(() => undefined);
    }
  }

  private subscribe(
    connection: NatsConnection,
    generation: number,
    subject: string,
    category: string,
    consume: (message: Pick<Msg, "subject" | "data">) => void | Promise<void>,
    isCurrent: () => boolean = () => true,
  ): Subscription {
    const owner =
      this.candidateGeneration?.id === generation
        ? this.candidateGeneration
        : this.activeGeneration;
    if (!owner) throw new Error("missing_connection_generation");
    const subscription = connection.subscribe(subject);
    void (async () => {
      try {
        let batchBytes = 0;
        let batchCount = 0;
        let deadline = Date.now() + 8;
        for await (const message of subscription) {
          if (!this.isLiveGeneration(generation) || !isCurrent()) return;
          if (batchCount === 0) deadline = Date.now() + 8;
          // Buffered NATS iteration advances via microtasks. Yield before the
          // next decrypt/decode burst so input can run, not just projection.
          if (batchCount >= 64 || batchBytes >= 256 * 1024 || Date.now() >= deadline) {
            await new Promise<void>(resolve => setTimeout(resolve, 0));
            if (!this.isLiveGeneration(generation)) return;
            batchBytes = 0; batchCount = 0; deadline = Date.now() + 8;
          }
          batchCount++;
          batchBytes += message.data.length;
          owner.deliver(() => {
            if (!this.isLiveGeneration(generation) || !isCurrent()) return;
            void Promise.resolve()
              .then(() => {
                if (!this.isLiveGeneration(generation) || !isCurrent()) return;
                const channel = this.secureChannels.get(connection);
                if (!channel) return;
                let data: Uint8Array;
                try { data = channel.open(message.subject, message.data); }
                catch { return; } // forged, replayed, or retired-channel event
                // MsgImpl exposes routing fields through prototype getters;
                // spreading it drops subject and makes state/events/transfers
                // throw after successful decryption, triggering reconnects.
                return consume({ subject: message.subject, data });
              })
              .catch((error) => this.failGeneration(error, generation));
          }, message.data.length);
          // An empty SDK queue already yields to network I/O. Do not introduce
          // a new wakeup for each slowly arriving token.
          if (subscription.getPending?.() === 0) { batchCount = 0; batchBytes = 0; }
        }
        if (isCurrent()) this.failGeneration(new Error(`remote_${category}_subscription_ended`), generation);
      } catch (error) {
        if (isCurrent()) this.failGeneration(error, generation);
      }
    })();
    return subscription;
  }

  private subscribeTransfers(connection: NatsConnection, generation: number): void {
    const prefix = `p.${this.credentials.pairId}.xfer.down.`;
    this.subscribe(connection, generation, `${prefix}>`, "transfer", (message) => {
      const parts = message.subject.slice(prefix.length).split(".");
      if (parts.length !== 3 || parts[1] !== "chunk") return;
      const key = `${parts[0]}:${parts[2]}`;
      const waiter = this.downloadWaiters.get(key);
      if (!waiter) return;
      clearTimeout(waiter.timer);
      this.downloadWaiters.delete(key);
      waiter.resolve(message.data);
    });
  }

  private subscribeEvents(connection: NatsConnection, generation: number, selective = false): void {
    const old = this.eventSubscriptions.get(connection);
    if (old) { old.revision++; old.subscription?.unsubscribe(); }
    this.eventSubscriptions.set(connection, { generation, selective, revision: 0 });
    this.installEventSubscription(connection);
  }

  private installEventSubscription(connection: NatsConnection): void {
    const record = this.eventSubscriptions.get(connection)!;
    const revision = ++record.revision;
    record.subscription?.unsubscribe();
    record.subscription = undefined;
    if (record.selective && !this.visibleSessionId) return;
    const prefix = `p.${this.credentials.pairId}.evt.`;
    record.subscription = this.subscribe(connection, record.generation,
      `${prefix}${record.selective ? this.visibleSessionId : ">"}`, "event", (message) => {
      const sessionId = message.subject.slice(prefix.length);
      let event: StreamEvent;
      try {
        event = decodeRemoteJson<StreamEvent>(message.data);
      } catch (error) {
        this.callbacks.onEventDecodeFailure(
          sessionId,
          errorWithContext("remote_event_decode_failed", error),
        );
        return;
      }
      this.callbacks.onEvent(event, sessionId);
    }, () => this.eventSubscriptions.get(connection) === record && record.revision === revision);
  }

  private subscribeLiveness(connection: NatsConnection, generation: number): void {
    this.subscribe(
      connection,
      generation,
      `p.${this.credentials.pairId}.presence`,
      "presence",
      async (message) => {
        let presence: Presence;
        try {
          presence = decodeRemoteJson<Presence>(message.data);
        } catch {
          return;
        }
        if (
          !presence.bridgeInstanceId ||
          presence.bridgeInstanceId !== this.confirmedBridgeInstanceId
        ) {
          const confirmation = await this.ensureHandshake(connection);
          await this.activateSecureChannel(connection);
          if (!this.isLiveGeneration(generation) || connection !== this.connection) return;
          this.confirmedBridgeInstanceId = confirmation.bridgeInstanceId;
          this.callbacks.onCatalogEpoch?.(confirmation.presence.catalogEpoch);
          this.receivePresence(confirmation.presence);
          this.negotiatedFeatures = new Set(confirmation.features ?? []);
          const selective = this.negotiatedFeatures.has("selective_events_v1");
          if (this.eventSubscriptions.get(connection)?.selective !== selective) this.subscribeEvents(connection, generation, selective);
          this.callbacks.onFeatures(confirmation.features ?? []);
          this.callbacks.onReconnected();
        } else this.receivePresence(presence);
      },
    );
  }

  private subscribeState(connection: NatsConnection, generation: number): void {
    const prefix = `p.${this.credentials.pairId}.state.`;
    this.subscribe(connection, generation, `${prefix}>`, "state", (message) => {
      const suffix = message.subject.slice(prefix.length);
      if (suffix === "events" && this.eventSubscriptions.get(connection)?.selective) {
        const event = decodeRemoteJson<StreamEvent & { sessionId: string }>(message.data);
        if (typeof event.sessionId === "string") this.callbacks.onEvent(event, event.sessionId);
      } else if (suffix === "sessions") {
        const data = decodeRemoteJson<{ sessions?: PresenceSession[]; version?: SnapshotVersion }>(
          message.data,
        );
        this.callbacks.onSessions(data.sessions ?? [], data.version);
      } else if (suffix === "workspaces") {
        const data = decodeRemoteJson<{
          workspaces?: RemoteWorkspace[];
          version?: SnapshotVersion;
        }>(message.data);
        this.callbacks.onWorkspaces(data.workspaces ?? [], data.version);
      }
    });
  }

  /**
   * The NATS status loop is the transport-truth feeder. SDK auto-reconnect is
   * disabled, so a disconnect disposes this generation and immediately asks
   * the local supervisor to build a replacement. Any natural loop exit also
   * falls back to `open_failed`; the same supervisor then owns the bounded
   * backoff and the three-minute recovery deadline.
   */
  private watchStatus(connection: NatsConnection, generation: number): void {
    void (async () => {
      let exitedNaturally = true;
      try {
        for await (const status of connection.status()) {
          if (!this.isLiveGeneration(generation)) {
            exitedNaturally = false;
            break;
          }
          if (status.type === "disconnect") {
            this.signal({ type: "transport_disconnect" });
            this.disposeConnection("network_unavailable");
            void this.open();
            return;
          } else if (status.type === "reconnect") {
            if (this.connection !== connection) continue;
            try {
              const confirmation = await this.ensureHandshake(connection);
              if (!this.isLiveGeneration(generation)) {
                exitedNaturally = false;
                break;
              }
              this.confirmedBridgeInstanceId = confirmation.bridgeInstanceId;
              await withTimeout(connection.flush(), 10_000);
              await this.activateSecureChannel(connection);
              if (!this.isLiveGeneration(generation) || this.connection !== connection) return;
              this.callbacks.onCatalogEpoch?.(confirmation.presence.catalogEpoch);
              this.receivePresence(confirmation.presence);
              this.negotiatedFeatures = new Set(confirmation.features ?? []);
              this.callbacks.onFeatures(confirmation.features ?? []);
              this.signal({ type: "ready" });
              this.callbacks.onReconnected();
            } catch (error) {
              this.failGeneration(error, generation);
              exitedNaturally = false;
              return;
            }
          } else if (status.type === "error") {
            const code = status.error.message;
            const kind = classifyNatsError(status.error);
            if (kind === "expired") {
              this.recordFailure("credential_expired", code);
              if (this.state !== "refreshing") void this.refreshToken();
            } else if (kind === "authorization") {
              this.recordFailure("service_authorization", code);
              this.handleFailure(new Error(`nats_authorization_rejected: ${code}`));
              return;
            } else if (kind === "protocol") {
              this.recordFailure("protocol", code);
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
      if (exitedNaturally && this.isLiveGeneration(generation)) {
        this.failGeneration(new Error("nats_connection_exhausted"), generation);
      }
    })();
  }

  private scheduleRefresh(): void {
    if (this.isTerminal()) return;
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
      if (this.isTerminal()) return;
      void this.refreshToken();
    }, delay);
  }

  get accessIdentity(): string | undefined {
    return this.confirmedBridgeInstanceId || undefined;
  }

  async request<T>(
    command: RemoteCommand,
    sessionId = command.sessionId ?? "list",
    timeoutMs = 10_000,
  ): Promise<RpcResponse<T>> {
    this.assertBusinessReady();
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
    const stableCommand = {
      bridgeInstanceId: this.confirmedBridgeInstanceId || undefined,
      ...command,
      id: command.id ?? randomId("cmd"),
    };
    let lastError: unknown;
    const retryable =
      command.type.startsWith("get_") ||
      command.type.startsWith("list_") ||
      command.type === "upload_complete" ||
      (["prompt", "continue_run"].includes(command.type) &&
        this.negotiatedFeatures.has("prompt_receipt_v1"));
    const attempts = retryable ? 3 : 1;
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      try {
        return await this.request<T>(stableCommand, sessionId);
      } catch (error) {
        lastError = error;
        if (!isTransientNatsRequestError(error)) throw error;
        if (attempt < attempts - 1) {
          // A retry on the same half-open generation only spends another
          // timeout. Validate/rebuild the transport first; the stable command
          // id keeps a reply lost during the swap idempotent on the desktop.
          await this.recoverAfterTransientRequest(error);
          await new Promise((resolve) => setTimeout(resolve, 250 * 2 ** attempt));
        }
      }
    }
    throw lastError;
  }

  async uploadChunk(transferId: string, index: number, bytes: Uint8Array): Promise<void> {
    this.assertBusinessReady();
    const connection = this.connection;
    if (!connection) throw new Error("not_connected");
    const message = await this.secureRequest(connection,
      `p.${this.credentials.pairId}.xfer.up.${transferId}.chunk.${index}`, bytes, 15_000);
    const response = decodeRemoteJson<RpcResponse>(message.data);
    if (!response.success) throw new Error(response.error ?? "upload_chunk_failed");
  }

  async downloadChunk(transferId: string, index: number): Promise<Uint8Array> {
    this.assertBusinessReady();
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
      const message = await this.secureRequest(connection,
        `p.${this.credentials.pairId}.xfer.up.${transferId}.pull.${index}`, new Uint8Array(), 15_000);
      const response = decodeRemoteJson<RpcResponse>(message.data);
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
    const payload = {
      bridgeInstanceId: this.confirmedBridgeInstanceId || undefined,
      ...command,
      id: command.id ?? randomId("cmd"),
    };
    const message = await this.secureRequest(connection,
      `p.${this.credentials.pairId}.cmd.${sessionId || "new"}`, encoder.encode(JSON.stringify(payload)), timeoutMs);
    // A valid reply already received on the previous serving socket remains
    // a valid command acknowledgement during a candidate handoff. Callers own
    // session/epoch fencing; do not recast a successful command as a failure.
    const response = await decodeRemoteJsonAsync<RpcResponse<T>>(message.data, () => !this.stopped);
    if (!response.success) {
      const error = new RemoteResponseError(response.error ?? "command_failed");
      throw error;
    }
    return response;
  }

  private async activateSecureChannel(connection: NatsConnection): Promise<void> {
    // Declare this client's capabilities. Both are opt-in so the desktop keeps
    // its legacy behaviour for a client that never asks:
    //   event_coalescing_v1 — the desktop may merge a run's text fragments, and
    //     this client reads a coalesced index range instead of one step per
    //     event (see `nextEvent`'s `coalescedCount`).
    //   reply_gzip_v1 — replies may be gzip-compressed; `decodeRemoteJson`
    //     detects the magic bytes. An older client does not have that check and
    //     would fail to parse a compressed reply, hence the declaration.
    //   lean_events_v1 — the desktop may omit reasoning text, streamed tool
    //     arguments and captured tool output. This client renders a reasoning
    //     row from its `thinking_start`/`thinking_end` boundary alone, takes a
    //     tool's target from `tool_start`'s complete arguments, and reads a
    //     tool's outcome from `exit_code`/`error` instead of parsing an
    //     `[exit: N]` footer out of the output. An older client does all three
    //     from the text, so it must keep receiving the full lane.
    await this.requestWithConnection(
      connection,
      {
        type: "secure_ready",
        features: ["event_coalescing_v1", "reply_gzip_v1", "lean_events_v1"],
      },
      "handshake",
    );
  }

  private async secureRequest(connection: NatsConnection, subject: string, plaintext: Uint8Array, timeout: number): Promise<Pick<Msg, "data">> {
    const channel = this.secureChannels.get(connection);
    if (!channel) throw new Error("pairing_handshake_required");
    const wire = channel.seal(subject, plaintext);
    const response = await connection.request(subject, wire, { timeout });
    return { data: channel.open(replyContext(subject, wire), response.data) };
  }

  private async performHandshake(connection: NatsConnection): Promise<HandshakeConfirmation> {
    if (!this.credentials.secureBundle) throw new Error("pairing_identity_mismatch");
    const bundle = JSON.parse(this.credentials.secureBundle) as { identity: SecureIdentity; desktopKey: string; secret?: string };
    const exchange = async (body: Record<string, unknown>) => {
      const response = await connection.request(`p.${this.credentials.pairId}.cmd.handshake`, encoder.encode(JSON.stringify(body)), { timeout: 10_000 });
      const parsed = decodeRemoteJson<RpcResponse<{ message?: string; id?: string; confirmation?: string }>>(response.data);
      // Keep the desktop's own refusal detail appended to the stable
      // `pairing_signature_invalid` token (which the credential classifier and
      // the connection presentation both match on): `invitation_already_used`,
      // `invitation_expired`, `peer_mismatch`, ... It is the only statement of
      // *why* a code the phone holds was turned down, and discarding it left
      // both the user and the phone console with nothing to act on.
      if (!parsed.success) {
        const detail = (parsed.error ?? "unknown").trim() || "unknown";
        throw new Error(`pairing_signature_invalid:${detail}`);
      }
      return parsed.data;
    };
    const run = async (secret?: string): Promise<HandshakeConfirmation> => {
      const handshake = new SecureHandshake(bundle.identity, this.credentials.pairId,
        this.credentials.expectedDesktopId, bundle.desktopKey, secret);
      let channel: SecureChannel | undefined;
      try {
        let response = await exchange({ type: "secure_open", pairing: Boolean(secret), message: encodeBase64Url(handshake.write()) });
        const message = response.message && decodeBase64Url(response.message);
        if (!message) throw new Error("pairing_signature_invalid");
        handshake.read(message);
        if (secret) response = await exchange({ type: "secure_finish", id: response.id, message: encodeBase64Url(handshake.write()) });
        channel = handshake.finish();
        const encrypted = response.confirmation && decodeBase64Url(response.confirmation);
        if (!encrypted) throw new Error("pairing_confirmation_mismatch");
        const confirmation = decodeRemoteJson<HandshakeConfirmation>(channel.open("handshake-confirm", encrypted));
        if (!confirmation.confirmed || confirmation.pairId !== this.credentials.pairId || !confirmation.bridgeInstanceId ||
            confirmation.presence.bridgeInstanceId !== confirmation.bridgeInstanceId || !confirmation.features?.includes("e2ee_v2")) {
          throw new Error("pairing_confirmation_mismatch");
        }
        if (this.stopped || (this.candidateConnection !== connection && this.connection !== connection)) throw new Error("not_connected");
        if (bundle.secret) {
          const credentials = { ...this.credentials, secureBundle: JSON.stringify({ identity: bundle.identity, desktopKey: bundle.desktopKey }) };
          await this.callbacks.onCredentials(credentials);
          if (this.stopped || (this.candidateConnection !== connection && this.connection !== connection)) throw new Error("not_connected");
          this.credentials = credentials;
        }
        this.secureChannels.get(connection)?.destroy();
        this.secureChannels.set(connection, channel);
        channel = undefined;
        return confirmation;
      } finally { handshake.destroy(); channel?.destroy(); }
    };
    // A first-pair response can be lost after Desktop durably binds this key.
    // IK proves that same local identity; it is never a plaintext/TOFU fallback.
    if (bundle.secret) {
      try { return await run(bundle.secret); }
      catch (error) { if (this.stopped) throw error; return run(); }
    }
    return run();
  }

  private ensureHandshake(connection: NatsConnection): Promise<HandshakeConfirmation> {
    const existing = this.handshakes.get(connection);
    if (existing) return existing;
    const pending = this.performHandshake(connection);
    this.handshakes.set(connection, pending);
    const clear = () => {
      if (this.handshakes.get(connection) === pending) this.handshakes.delete(connection);
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

/** A local readiness gate postpones delivery, but must not trigger an RPC
 * retry loop. Keep the pending operation until business readiness returns. */
export function isDeferredRequestError(error: unknown): boolean {
  return !(error instanceof RemoteResponseError) &&
    error instanceof Error && error.message === "communication_frozen";
}

export function isTransientNatsRequestError(error: unknown): boolean {
  if (error instanceof RemoteResponseError) return false;
  if (error instanceof Error && error.message === "not_connected") return true;
  return classifyNatsError(error) === "transport";
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
