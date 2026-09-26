import type { Msg, NatsConnection } from "@nats-io/nats-core";
import { wsconnect } from "@nats-io/nats-core";
import { gzipSync } from "fflate";
import { RemoteClient, type RemoteClientCallbacks } from "../client";
import { isDeferredRequestError, isTransientNatsRequestError } from "../client";
import { RemoteApiError } from "../connectionState";
import { classifyNatsError } from "../natsErrors";
import { jwtExpiry } from "../codec";
import { ensureFreshCredentials, refreshCredentials } from "../pairing";
import { decodeRemoteJson } from "../remoteJson";
import type { RemoteCredentials } from "../types";

jest.mock("@nats-io/nats-core", () => ({
  wsconnect: jest.fn(),
  jwtAuthenticator: jest.fn(),
}));
jest.mock("../natsErrors", () => ({ classifyNatsError: jest.fn(() => "transport") }));
jest.mock("../pairing", () => ({
  ensureFreshCredentials: jest.fn(),
  refreshCredentials: jest.fn(),
}));
jest.mock("../codec", () => ({
  ...jest.requireActual("../codec"),
  jwtExpiry: jest.fn(() => Math.floor(Date.now() / 1000) + 3600),
  randomId: () => "id",
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

class Messages<T> implements AsyncIterable<T> {
  private queue: T[] = [];
  private waiting: (() => void) | null = null;
  private closed = false;
  /** The real NATS `Subscription` exposes `unsubscribe()`; the fake models it so
   * the client's reinstall path (which calls it when a lane is replaced) is
   * exercised faithfully instead of short-circuiting on a missing method. */
  unsubscribe = jest.fn();
  push(value: T) {
    // A burst can arrive while the consumer is suspended at its own yield
    // point, so pushes are queued rather than overwriting one waiting slot
    // (which silently dropped every frame but the last).
    this.queue.push(value);
    const waiting = this.waiting;
    this.waiting = null;
    waiting?.();
  }
  end() {
    this.closed = true;
    const waiting = this.waiting;
    this.waiting = null;
    waiting?.();
  }
  private async take(): Promise<IteratorResult<T>> {
    while (this.queue.length === 0 && !this.closed) {
      await new Promise<void>(resolve => {
        this.waiting = resolve;
      });
    }
    const value = this.queue.shift();
    if (value === undefined) return { done: true, value: undefined };
    return { done: false, value };
  }
  [Symbol.asyncIterator]() {
    return { next: () => (this.closed && this.queue.length === 0
      ? Promise.resolve({ done: true as const, value: undefined })
      : this.take()) };
  }
}

function socket() {
  const subscriptions = new Map<string, Messages<Msg>>();
  const status = new Messages<never>();
  const connection = {
    subscribe: jest.fn((subject: string) => {
      const messages = new Messages<Msg>();
      subscriptions.set(subject, messages);
      return messages;
    }),
    status: () => status,
    request: jest.fn(async () => ({ data: new TextEncoder().encode(JSON.stringify({ success: true, data: {
      pairId: "pair", bridgeInstanceId: "bridge", online: true, lastHeartbeatTs: Date.now() / 1000,
    } })) })),
    flush: jest.fn(async () => {}),
    isClosed: () => connection.close.mock.calls.length > 0,
    close: jest.fn(async () => {
      for (const messages of subscriptions.values()) messages.end();
      status.end();
    }),
  };
  return {
    connection: connection as unknown as NatsConnection,
    close: connection.close,
    flush: connection.flush,
    subscriptions,
  };
}

const credentials = {
  pairId: "pair",
  seed: "seed",
  userJwt: "jwt",
  natsWsUrl: "wss://example.test",
  deviceId: "dev",
} as RemoteCredentials;
const encoder = new TextEncoder();
const tick = async () => {
  for (let n = 0; n < 20; n++) await Promise.resolve();
};

describe("RemoteClient connection handoff", () => {
  let client: RemoteClient;
  let callbacks: RemoteClientCallbacks;
  const confirmation = {
    bridgeInstanceId: "bridge",
    presence: { bridgeInstanceId: "bridge", online: true },
  };

  beforeEach(() => {
    jest.useFakeTimers();
    jest.clearAllMocks();
    jest.spyOn(console, "warn").mockImplementation(() => {});
    (ensureFreshCredentials as jest.Mock).mockResolvedValue(credentials);
    callbacks = {
      onCredentials: jest.fn(async () => {}),
      onEvent: jest.fn(),
      onEventDecodeFailure: jest.fn(),
      onPresence: jest.fn(),
      onSessions: jest.fn(),
      onWorkspaces: jest.fn(),
      onFeatures: jest.fn(),
      onConnectionState: jest.fn(),
      onReconnected: jest.fn(),
      onError: jest.fn(),
    };
    client = new RemoteClient(credentials, callbacks);
    // Keep the real lifecycle and subscription loops; cryptographic identity
    // validation has its own handshake suite.
    jest.spyOn(client as never, "performHandshake").mockResolvedValue(confirmation as never);
    jest.spyOn(client as never, "activateSecureChannel").mockResolvedValue(undefined as never);
    // This suite mocks the handshake, so mock its installed record layer too.
    // secureClient.test.ts exercises the actual cryptographic boundary.
    const boundary = client as unknown as { secureChannels: WeakMap<object, unknown>; secureRequest: (connection: NatsConnection, subject: string, bytes: Uint8Array, timeout: number) => Promise<Msg> };
    jest.spyOn(boundary.secureChannels, "get").mockReturnValue({ open: (_: string, bytes: Uint8Array) => bytes, destroy: () => {} });
    boundary.secureRequest = (connection, subject, bytes, timeout) => connection.request(subject, bytes, { timeout });
  });

  afterEach(async () => {
    await client.close();
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  test.each(["suspended", "running"])("long background does not consume recovery budget (%s timers)", async mode => {
    (wsconnect as jest.Mock).mockImplementation(async () => socket().connection);
    await client.open();
    client.setAppActive(false);
    // Native reachability notifications must not start a background budget.
    await client.recoverNow("network-restored");
    await client.open();
    if (mode === "suspended") jest.setSystemTime(Date.now() + 60 * 60_000);
    else await jest.advanceTimersByTimeAsync(60 * 60_000);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    expect(wsconnect).toHaveBeenCalledTimes(2);
    expect(callbacks.onError).not.toHaveBeenCalled();
  });

  test("foreground resumes a bounded budget even without a completed attempt", async () => {
    (wsconnect as jest.Mock).mockResolvedValue(socket().connection);
    await client.open();
    client.setAppActive(false);
    jest.setSystemTime(Date.now() + 60 * 60_000);
    client.setAppActive(true);
    await jest.advanceTimersByTimeAsync(179_999);
    expect(callbacks.onConnectionState).not.toHaveBeenLastCalledWith("failed");
    await jest.advanceTimersByTimeAsync(1);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    client.setAppActive(false);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });

  test("initial connection budget does not expire while the app starts in the background", async () => {
    (wsconnect as jest.Mock).mockResolvedValue(socket().connection);
    client.setAppActive(false);
    await client.open();
    await jest.advanceTimersByTimeAsync(60 * 60_000);
    expect(wsconnect).not.toHaveBeenCalled();
    client.setAppActive(true);
    await client.open();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    expect(callbacks.onError).not.toHaveBeenCalled();
  });

  test("explicit close cannot be undone by a long background and foreground transition", async () => {
    (wsconnect as jest.Mock).mockResolvedValue(socket().connection);
    await client.open();
    await client.close();
    client.setAppActive(false);
    jest.setSystemTime(Date.now() + 60 * 60_000);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });

  test("initial network failure is terminal and does not retry", async () => {
    (wsconnect as jest.Mock).mockRejectedValue(new Error("network down"));
    await client.open();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    await jest.advanceTimersByTimeAsync(180_000);
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });

  test("real offline recovery retries within three minutes, then stays terminal", async () => {
    (wsconnect as jest.Mock).mockResolvedValueOnce(socket().connection).mockRejectedValue(new Error("network down"));
    await client.open();
    client.setAppActive(false);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    await expect(client.request({ type: "list_sessions" })).rejects.toThrow("communication_frozen");
    await jest.advanceTimersByTimeAsync(180_000);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    expect(callbacks.onError).toHaveBeenLastCalledWith(new Error("recovery_timeout"));
    const attempts = jest.mocked(wsconnect).mock.calls.length;
    expect(attempts).toBeGreaterThan(2);
    (wsconnect as jest.Mock).mockResolvedValue(socket().connection);
    await client.recoverNow("foreground");
    await client.recoverNow("network-restored");
    await jest.advanceTimersByTimeAsync(60_000);
    expect(wsconnect).toHaveBeenCalledTimes(attempts);
  });

  test("foreground attempts immediately, then retries at 1s, 2s, 4s without network events", async () => {
    jest.spyOn(Math, "random").mockReturnValue(0.5);
    (wsconnect as jest.Mock).mockResolvedValueOnce(socket().connection).mockRejectedValue(new Error("network down"));
    await client.open();
    client.setAppActive(false);
    jest.setSystemTime(Date.now() + 60 * 60_000);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    expect(wsconnect).toHaveBeenCalledTimes(2);
    for (const [delay, attempts] of [[1_000, 3], [2_000, 4], [4_000, 5]] as const) {
      await jest.advanceTimersByTimeAsync(delay - 1);
      expect(wsconnect).toHaveBeenCalledTimes(attempts - 1);
      await jest.advanceTimersByTimeAsync(1);
      expect(wsconnect).toHaveBeenCalledTimes(attempts);
    }
    // Actual reachability recovers, but the OS never sends a notification.
    (wsconnect as jest.Mock).mockResolvedValue(socket().connection);
    await jest.advanceTimersByTimeAsync(8_000);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    expect(wsconnect).toHaveBeenCalledTimes(6);
  });

  test("network-restored accelerates backoff and does not leave a duplicate retry", async () => {
    jest.spyOn(Math, "random").mockReturnValue(0.5);
    (wsconnect as jest.Mock).mockResolvedValueOnce(socket().connection).mockRejectedValue(new Error("network down"));
    await client.open();
    client.setAppActive(false);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    await jest.advanceTimersByTimeAsync(1_000);
    expect(wsconnect).toHaveBeenCalledTimes(3);
    (wsconnect as jest.Mock).mockResolvedValue(socket().connection);
    await client.recoverNow("network-restored");
    expect(wsconnect).toHaveBeenCalledTimes(4);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    await jest.advanceTimersByTimeAsync(2_000);
    expect(wsconnect).toHaveBeenCalledTimes(4);
  });

  test("network notifications during a pending retry share the same connection attempt", async () => {
    jest.spyOn(Math, "random").mockReturnValue(0.5);
    (wsconnect as jest.Mock).mockResolvedValueOnce(socket().connection).mockRejectedValue(new Error("network down"));
    await client.open();
    client.setAppActive(false);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    const pending = deferred<NatsConnection>();
    (wsconnect as jest.Mock).mockReturnValueOnce(pending.promise);
    await jest.advanceTimersByTimeAsync(1_000);
    const first = client.recoverNow("network-restored");
    const second = client.recoverNow("network-changed");
    await tick();
    expect(wsconnect).toHaveBeenCalledTimes(3);
    pending.resolve(socket().connection);
    await Promise.all([first, second]);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    expect(wsconnect).toHaveBeenCalledTimes(3);
  });

  test.each(["background", "close", "unpair"])("%s cancels pending backoff and ignores network-restored", async stop => {
    (wsconnect as jest.Mock).mockResolvedValueOnce(socket().connection).mockRejectedValue(new Error("network down"));
    await client.open();
    client.setAppActive(false);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    expect(wsconnect).toHaveBeenCalledTimes(2);
    if (stop === "background") client.setAppActive(false);
    else await client.close(stop === "unpair" ? "Unpair" : "UserInitiated");
    await client.recoverNow("network-restored");
    await jest.advanceTimersByTimeAsync(180_000);
    expect(wsconnect).toHaveBeenCalledTimes(2);
  });

  test("revoked credentials stop foreground retries and ignore later network recovery", async () => {
    (wsconnect as jest.Mock).mockResolvedValue(socket().connection);
    await client.open();
    client.setAppActive(false);
    client.setAppActive(true);
    (ensureFreshCredentials as jest.Mock).mockRejectedValue(new RemoteApiError("revoked", "credentials_revoked", 401));
    await client.recoverNow("foreground");
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("revoked");
    (ensureFreshCredentials as jest.Mock).mockResolvedValue(credentials);
    await client.recoverNow("network-restored");
    await jest.advanceTimersByTimeAsync(180_000);
    expect(wsconnect).toHaveBeenCalledTimes(1);
    expect(ensureFreshCredentials).toHaveBeenCalledTimes(2);
  });

  test("initial timeout rejects a late credential result", async () => {
    const fresh = deferred<RemoteCredentials>();
    (ensureFreshCredentials as jest.Mock).mockReturnValue(fresh.promise);
    const opening = client.open();
    await jest.advanceTimersByTimeAsync(20_000);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    fresh.resolve(credentials);
    await opening;
    expect(wsconnect).not.toHaveBeenCalled();
  });

  test("credential refresh pending does not retire the serving event subscription", async () => {
    const old = socket();
    const replacement = socket();
    (wsconnect as jest.Mock)
      .mockResolvedValueOnce(old.connection)
      .mockResolvedValueOnce(replacement.connection);
    await client.open();
    const fresh = deferred<RemoteCredentials>();
    (ensureFreshCredentials as jest.Mock).mockReturnValueOnce(fresh.promise);
    const recovering = client.recoverNow("presence-stale");
    await tick();
    old.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.session",
      data: new TextEncoder().encode('{"type":"agent_end"}'),
    } as Msg);
    await tick();
    expect(callbacks.onEvent).toHaveBeenCalledWith({ type: "agent_end" }, "session");
    expect(old.close).not.toHaveBeenCalled();
    fresh.resolve(credentials);
    await recovering;
    expect(old.close).toHaveBeenCalledTimes(1);
    expect(callbacks.onReconnected).toHaveBeenCalledTimes(2);
  });

  test("failed candidate readiness keeps the old connection usable", async () => {
    const old = socket();
    const replacement = socket();
    (wsconnect as jest.Mock)
      .mockResolvedValueOnce(old.connection)
      .mockResolvedValueOnce(replacement.connection);
    await client.open();
    replacement.flush.mockRejectedValueOnce(new Error("network timeout"));
    await client.recoverNow("presence-stale");
    expect(replacement.close).toHaveBeenCalled();
    expect(old.close).not.toHaveBeenCalled();
    old.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.session",
      data: new TextEncoder().encode('{"type":"agent_end"}'),
    } as Msg);
    await tick();
    expect(callbacks.onEvent).toHaveBeenCalledTimes(1);
    // Receiving catalog/events through the still-serving channel must not
    // leave the UI yellow with every conversation action disabled.
    expect(old.connection.request).toHaveBeenCalled();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("one-way event delivery does not mark a broken command channel ready", async () => {
    const old = socket();
    const replacement = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(old.connection).mockResolvedValueOnce(replacement.connection);
    await client.open();
    replacement.flush.mockRejectedValueOnce(new Error("network timeout"));
    jest.mocked(old.connection.request).mockRejectedValueOnce(new Error("request timeout"));
    await client.recoverNow("presence-stale");
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("reconnecting");
  });

  test("a late successful fallback probe cannot resurrect a closed client", async () => {
    const old = socket();
    const replacement = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(old.connection).mockResolvedValueOnce(replacement.connection);
    await client.open();
    replacement.flush.mockRejectedValueOnce(new Error("network timeout"));
    const response = deferred<Msg>();
    jest.mocked(old.connection.request).mockReturnValueOnce(response.promise);
    const recovering = client.recoverNow("presence-stale");
    await tick();
    await client.close();
    jest.mocked(callbacks.onConnectionState).mockClear();
    response.resolve({ data: new TextEncoder().encode(JSON.stringify({ success: true, data: { online: true, pairId: "pair", bridgeInstanceId: "bridge" } })) } as Msg);
    await recovering;
    expect(callbacks.onConnectionState).not.toHaveBeenCalled();
  });

  test("foreground reuses a socket only after authenticated desktop presence succeeds", async () => {
    const old = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(old.connection);
    await client.open();
    await client.recoverNow("foreground");
    expect(old.connection.request).toHaveBeenCalledTimes(1);
    expect(wsconnect).toHaveBeenCalledTimes(1);
    expect(old.close).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("foreground rebuilds when the broker responds but the desktop channel is gone", async () => {
    const old = socket();
    const replacement = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(old.connection).mockResolvedValueOnce(replacement.connection);
    await client.open();
    jest.mocked(old.connection.request).mockRejectedValueOnce(new Error("request timeout"));
    await client.recoverNow("foreground");
    expect(old.flush).toHaveBeenCalled();
    expect(old.connection.request).toHaveBeenCalledTimes(1);
    expect(old.close).toHaveBeenCalledTimes(1);
    expect(wsconnect).toHaveBeenCalledTimes(2);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("background teardown releases a suspended recovery before foreground reconnects", async () => {
    const old = socket();
    const replacement = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(old.connection).mockResolvedValueOnce(replacement.connection);
    await client.open();
    const fresh = deferred<RemoteCredentials>();
    (ensureFreshCredentials as jest.Mock).mockReturnValueOnce(fresh.promise);
    const suspended = client.recoverNow("presence-stale");
    await tick();
    const signal = (ensureFreshCredentials as jest.Mock).mock.calls[1][1] as AbortSignal;
    client.setAppActive(false);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    expect(signal.aborted).toBe(true);
    expect(wsconnect).toHaveBeenCalledTimes(2);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    fresh.resolve(credentials);
    await suspended;
    expect(wsconnect).toHaveBeenCalledTimes(2);
    expect(replacement.close).not.toHaveBeenCalled();
  });

  test("stop cancels an outstanding credential request and ignores its late result", async () => {
    const fresh = deferred<RemoteCredentials>();
    (ensureFreshCredentials as jest.Mock).mockReturnValueOnce(fresh.promise);
    const opening = client.open();
    const signal = (ensureFreshCredentials as jest.Mock).mock.calls[0][1] as AbortSignal;
    await client.close();
    expect(signal.aborted).toBe(true);
    fresh.resolve(credentials);
    await opening;
    expect(wsconnect).not.toHaveBeenCalled();
    expect(callbacks.onReconnected).not.toHaveBeenCalled();
  });

  test("concurrent open calls share one candidate", async () => {
    (wsconnect as jest.Mock).mockResolvedValue(socket().connection);
    await Promise.all([client.open(), client.open(), client.recoverNow("network-changed")]);
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });
  test("candidate events wait for readiness and survive the handoff", async () => {
    const old = socket();
    const replacement = socket();
    (wsconnect as jest.Mock)
      .mockResolvedValueOnce(old.connection)
      .mockResolvedValueOnce(replacement.connection);
    await client.open();
    const ready = deferred<void>();
    replacement.flush.mockReturnValueOnce(ready.promise);
    const recovering = client.recoverNow("presence-stale");
    await tick();
    replacement.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.session",
      data: new TextEncoder().encode('{"type":"agent_end"}'),
    } as Msg);
    await tick();
    expect(callbacks.onEvent).not.toHaveBeenCalled();
    ready.resolve();
    await recovering;
    await tick();
    expect(callbacks.onEvent).toHaveBeenCalledTimes(1);
  });

  test("stop during candidate readiness closes both sockets and prevents ready", async () => {
    const old = socket();
    const replacement = socket();
    (wsconnect as jest.Mock)
      .mockResolvedValueOnce(old.connection)
      .mockResolvedValueOnce(replacement.connection);
    await client.open();
    const ready = deferred<void>();
    replacement.flush.mockReturnValueOnce(ready.promise);
    const recovering = client.recoverNow("presence-stale");
    await tick();
    await client.close();
    ready.resolve();
    await recovering;
    expect(old.close).toHaveBeenCalled();
    expect(replacement.close).toHaveBeenCalled();
    expect(callbacks.onReconnected).toHaveBeenCalledTimes(1);
  });
  test.each(["delete_workspace", "prompt", "continue_run"])(
    "does not automatically retry %s without a durable retry contract",
    async (type) => {
      const request = jest.spyOn(client, "request").mockRejectedValue(new Error("timeout"));
      await expect(client.requestRetry({ type })).rejects.toThrow("timeout");
      expect(request).toHaveBeenCalledTimes(1);
    },
  );

  // The desktop only gzips replies to a connection that declares it, and only
  // merges a run's fragments for a connection that can read a coalesced index
  // range. Dropping a capability here silently loses the optimisation; adding
  // one this client cannot decode would break every reply it receives.
  test("declares exactly the capabilities this client can decode", async () => {
    const open = socket();
    // A fresh client: the shared one has `activateSecureChannel` stubbed out by
    // the suite setup, and this test needs the real declaration.
    const declaring = new RemoteClient(credentials, callbacks);
    const calls: { command: unknown; sessionId?: string }[] = [];
    jest
      .spyOn(declaring as never, "requestWithConnection" as never)
      .mockImplementation((async (
        _connection: unknown,
        command: unknown,
        sessionId?: string,
      ) => {
        calls.push({ command, sessionId });
        return { success: true, data: undefined };
      }) as never);
    await (declaring as unknown as {
      activateSecureChannel: (connection: unknown) => Promise<void>;
    }).activateSecureChannel(open.connection);
    expect(calls).toHaveLength(1);
    const declaration = calls[0];
    expect(declaration).toBeDefined();
    expect(declaration?.sessionId).toBe("handshake");
    expect(declaration?.command).toEqual({
      type: "secure_ready",
      features: ["event_coalescing_v1", "reply_gzip_v1", "lean_events_v1"],
    });
    // Each declared capability has to stay truthful for this build, or the
    // declaration must be dropped with it. The lean feed's three requirements
    // are asserted in the projection tests: a reasoning row survives with no
    // text, a tool target comes from `tool_start`, and a non-zero `exit_code`
    // with no output still fails the row.
    // `reply_gzip_v1` is only truthful while the decoder sniffs the gzip magic.
    // If this ever stops holding, the declaration above must be removed too.
    const compressed = gzipSync(new TextEncoder().encode(JSON.stringify({ ok: true })));
    expect(compressed[0]).toBe(0x1f);
    expect(decodeRemoteJson<{ ok: boolean }>(compressed)).toEqual({ ok: true });
  });

  test("the visible session is validated before it reaches a wildcard subject", () => {
    const install = jest.spyOn(client as never, "installEventSubscription");
    const internals = client as unknown as { visibleSessionId: string };
    // A session id is interpolated into a NATS subject: anything outside the
    // agent's own id alphabet is dropped rather than escaped into a pattern.
    client.setVisibleSession("../../>");
    expect(internals.visibleSessionId).toBe("");
    client.setVisibleSession("session_1-2");
    expect(internals.visibleSessionId).toBe("session_1-2");
    // Only a *change* reinstalls: a repeated call must not duplicate the SUB.
    client.setVisibleSession("session_1-2");
    expect(install).not.toHaveBeenCalled();
  });

  test("changing the visible session reinstalls only a selective subscription", async () => {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    jest.spyOn(client as never, "performHandshake").mockResolvedValue({
      bridgeInstanceId: "bridge",
      features: ["selective_events_v1"],
      presence: { bridgeInstanceId: "bridge", online: true },
    } as never);
    const install = jest.spyOn(client as never, "installEventSubscription");
    await client.open();
    install.mockClear();
    client.setVisibleSession("session_a");
    // The live connection carries the filter, so a visible-session change has
    // to reinstall exactly that subscription.
    expect(install).toHaveBeenCalledTimes(1);
    expect(install).toHaveBeenCalledWith(live.connection);
  });

  test("a desktop that stops refreshing presence is presumed gone and re-probed", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => socket().connection);
    await client.open();
    const recover = jest.spyOn(client, "recoverNow").mockResolvedValue(undefined);
    const internals = client as unknown as {
      receivePresence(p: unknown): void;
      desktopAvailable: boolean;
    };
    internals.receivePresence({ agentAvailable: true, bridgeInstanceId: "bridge", online: true });
    expect(internals.desktopAvailable).toBe(true);
    // 15 s without a heartbeat: a killed desktop would otherwise leave every
    // command hanging on a peer that is no longer there.
    await jest.advanceTimersByTimeAsync(15_000);
    expect(internals.desktopAvailable).toBe(false);
    expect(recover).toHaveBeenCalledWith("network-changed");
  });

  test("an explicitly disconnected desktop is not put on a liveness timer", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => socket().connection);
    await client.open();
    const internals = client as unknown as {
      receivePresence(p: unknown): void;
      desktopAvailable: boolean;
    };
    internals.receivePresence({ bridgeInstanceId: "bridge", disconnected: true, online: true });
    // A shutdown the desktop announced itself is not a stale heartbeat, so no
    // expiry is armed: the state stays exactly as reported.
    expect(internals.desktopAvailable).toBe(true);
    await jest.advanceTimersByTimeAsync(60_000);
    expect(internals.desktopAvailable).toBe(true);
  });

  test("a desktop that answers without a usable agent is not business-ready", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => socket().connection);
    await client.open();
    const internals = client as unknown as {
      receivePresence(p: unknown): void;
      assertBusinessReady(): void;
    };
    internals.receivePresence({ agentAvailable: false, bridgeInstanceId: "bridge", online: true });
    // The socket is up but the agent inside the desktop is not: a command would
    // hang, so the client refuses it up front instead of timing out later.
    expect(() => internals.assertBusinessReady()).toThrow("communication_frozen");
  });

  test("a recovery budget that already ran out fails the connection instead of waiting again", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => socket().connection);
    await client.open();
    const internals = client as unknown as {
      everReady: boolean;
      recoveryDeadline: number;
      beginDeadline(): boolean;
    };
    internals.everReady = true;
    // A deadline that expired while JS was suspended (an OS wake, a long GC
    // pause) ends the attempt now rather than scheduling another fruitless wait.
    internals.recoveryDeadline = Date.now() - 1;
    expect(internals.beginDeadline()).toBe(false);
    expect(callbacks.onError).toHaveBeenCalledWith(expect.objectContaining({
      message: "recovery_timeout",
    }));
  });

  test("a network event after the client was closed cannot reopen it", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => socket().connection);
    await client.open();
    await client.close();
    await client.recoverNow("network-restored");
    // A closed client is terminal: a queued OS notification must not reopen a
    // socket the user already released.
    expect(wsconnect).toHaveBeenCalledTimes(1);
    await expect(client.request({ type: "list_sessions" })).rejects.toThrow("communication_frozen");
  });

  test("an error with no message and no code still gets a labelled, non-empty error", async () => {
    (wsconnect as jest.Mock).mockRejectedValue({ code: "  ", name: "Error" });
    await client.open();
    // A rejected object with no usable field must not surface as "undefined".
    const reported = jest.mocked(callbacks.onError).mock.calls.at(-1)?.[0];
    expect(reported).toBeInstanceOf(Error);
    expect(reported!.message).toContain("nats_connect_failed");
  });
});

/**
 * A generation that is abandoned mid-flight owns no timer and must tear its
 * own socket down: the phone can re-pair, background, or start a replacement
 * while a connect, a handshake, or a subscription teardown is still open.
 */
describe("dead generations clean up after themselves", () => {
  let client: RemoteClient;
  let callbacks: RemoteClientCallbacks;
  const confirmation = {
    bridgeInstanceId: "bridge",
    presence: { bridgeInstanceId: "bridge", online: true },
  };

  beforeEach(() => {
    jest.useFakeTimers();
    jest.clearAllMocks();
    jest.spyOn(console, "warn").mockImplementation(() => {});
    jest.mocked(ensureFreshCredentials).mockResolvedValue(credentials);
    callbacks = {
      onCredentials: jest.fn(async () => {}),
      onEvent: jest.fn(),
      onEventDecodeFailure: jest.fn(),
      onPresence: jest.fn(),
      onSessions: jest.fn(),
      onWorkspaces: jest.fn(),
      onFeatures: jest.fn(),
      onConnectionState: jest.fn(),
      onReconnected: jest.fn(),
      onError: jest.fn(),
    };
    client = new RemoteClient(credentials, callbacks);
    jest.spyOn(client as never, "performHandshake").mockResolvedValue(confirmation as never);
    jest.spyOn(client as never, "activateSecureChannel").mockResolvedValue(undefined as never);
    const boundary = client as unknown as {
      secureChannels: WeakMap<object, unknown>;
      secureRequest: (connection: NatsConnection, subject: string, bytes: Uint8Array, timeout: number) => Promise<Msg>;
    };
    jest.spyOn(boundary.secureChannels, "get").mockReturnValue({ open: (_: string, bytes: Uint8Array) => bytes, destroy: () => {} });
    boundary.secureRequest = (connection, subject, bytes, timeout) => connection.request(subject, bytes, { timeout });
  });

  afterEach(async () => {
    await client.close();
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  test("a close during the handshake drops the half-built socket instead of publishing it", async () => {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    const handshake = deferred<unknown>();
    jest.spyOn(client as never, "performHandshake").mockReturnValue(handshake.promise as never);
    const opening = client.open();
    await tick();
    expect(live.connection.close).not.toHaveBeenCalled();
    // The user unpairs (or the screen closes) while the handshake is in flight.
    await client.close();
    handshake.resolve(confirmation);
    await opening;
    // The socket must not survive its own client: an orphaned NATS connection
    // keeps a subscription on the broker and outlives the credentials.
    expect(live.connection.close).toHaveBeenCalled();
  });

  test("a close while the subscription barrier is flushing drops the candidate socket", async () => {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    const flushed = deferred<void>();
    (live.connection.flush as jest.Mock).mockReturnValue(flushed.promise);
    const opening = client.open();
    await tick();
    // The subscriptions are installed but not yet proven; closing here must not
    // leave a candidate generation holding a live broker socket.
    await client.close();
    flushed.resolve();
    await opening;
    expect(live.connection.close).toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("a candidate whose own subscription dies is failed and closed, not retried", async () => {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    const flushed = deferred<void>();
    (live.connection.flush as jest.Mock).mockReturnValue(flushed.promise);
    const opening = client.open();
    await tick();
    // The candidate generation is public while its barrier is in progress, so
    // its subscriptions are already attached (line 588-591).
    const state = live.subscriptions.get("p.pair.state.>");
    expect(state).toBeDefined();
    state!.end();
    await tick();
    flushed.resolve();
    await opening;
    // A candidate that died before activation must be discarded — every live
    // generation would otherwise be retried as though it had served traffic.
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("a close during the socket connect drops the socket before any handshake", async () => {
    const live = socket();
    const connected = deferred<NatsConnection>();
    (wsconnect as jest.Mock).mockReturnValue(connected.promise);
    const opening = client.open();
    await tick();
    // The user unpairs while the WebSocket handshake is still in flight: the
    // socket that lands afterwards belongs to nobody.
    await client.close();
    connected.resolve(live.connection);
    await opening;
    expect(live.connection.close).toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("a live subscription that dies while a replacement is opening is not retried twice", async () => {
    const first = socket();
    const second = socket();
    (wsconnect as jest.Mock)
      .mockResolvedValueOnce(first.connection)
      .mockResolvedValue(second.connection);
    await client.open();
    // Start a replacement: `openPromise` is set for the whole attempt, which is
    // the window in which a dying subscription must not arm a second retry.
    const reconnect = deferred<unknown>();
    jest.spyOn(client as never, "performHandshake")
      .mockReturnValueOnce(reconnect.promise as never)
      .mockResolvedValue(confirmation as never);
    const attempt = client.open();
    await tick();
    (callbacks.onError as jest.Mock).mockClear();
    first.subscriptions.get("p.pair.state.>")!.end();
    await tick();
    // The open in flight already owns the retry. A second backoff timer here
    // is the reconnect storm this guard exists to prevent, so no error is
    // surfaced either — the user sees one reconnecting phase, not a failure.
    expect(callbacks.onError).not.toHaveBeenCalled();
    reconnect.resolve(confirmation);
    await attempt;
    expect(wsconnect).toHaveBeenCalledTimes(2);
    await jest.advanceTimersByTimeAsync(60_000);
    expect(wsconnect).toHaveBeenCalledTimes(2);
  });

  test("a consumer callback that throws fails its generation instead of losing the frame", async () => {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    (client as unknown as { receivePresence(p: unknown): void }).receivePresence({
      bridgeInstanceId: "bridge", online: true, agentAvailable: true,
    });
    const failure = new Error("projection exploded");
    (callbacks.onEvent as jest.Mock).mockImplementationOnce(() => { throw failure; });
    live.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.s1",
      data: encoder.encode(JSON.stringify({ type: "agent_end", data: "{}" })),
    } as never);
    await tick();
    // A throwing consumer means the projection is broken for this connection,
    // so the generation is failed and rebuilt rather than dropping every later
    // frame on the floor.
    expect(callbacks.onError).toHaveBeenCalledWith(failure);
  });

  test("a foreground probe that finds the desktop healthy keeps the socket and its retry is cancelled", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => socket().connection);
    await client.open();
    (client as unknown as { receivePresence(p: unknown): void }).receivePresence({
      bridgeInstanceId: "bridge", online: true, agentAvailable: true,
    });
    // A foreground recovery probes the authenticated command path before
    // reusing the socket (a broker PONG does not prove the desktop still holds
    // our traffic keys); the probe reply here matches, so the socket is kept.
    await client.recoverNow("foreground");
    expect(wsconnect).toHaveBeenCalledTimes(1);
    await jest.advanceTimersByTimeAsync(120_000);
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });

  test.each([
    ["another pair", { pairId: "other-pair" }],
    ["another bridge instance", { bridgeInstanceId: "other-bridge" }],
  ])("a foreground probe that answers for %s rebuilds the connection", async (_label, override) => {
    const first = socket();
    const second = socket();
    (wsconnect as jest.Mock)
      .mockResolvedValueOnce(first.connection)
      .mockResolvedValue(second.connection);
    await client.open();
    (client as unknown as { receivePresence(p: unknown): void }).receivePresence({
      bridgeInstanceId: "bridge", online: true, agentAvailable: true,
    });
    // The probe is answered \u2014 online, and by a live desktop \u2014 but not for this
    // pairing or this bridge process. A desktop that restarted keeps answering
    // PINGs while holding none of our traffic keys, so a matching "online" flag
    // is not enough: both identities have to agree before the socket is reused.
    (first.connection.request as jest.Mock).mockResolvedValue({
      data: encoder.encode(JSON.stringify({ success: true, data: {
        pairId: "pair", bridgeInstanceId: "bridge", online: true, lastHeartbeatTs: 0,
        ...override,
      } })),
    });
    await client.recoverNow("foreground");
    expect(second.connection.close).not.toHaveBeenCalled();
    expect(wsconnect).toHaveBeenCalledTimes(2);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("a pairing revoked while the replacement was activating is never published", async () => {
    const first = socket();
    const second = socket();
    (wsconnect as jest.Mock)
      .mockResolvedValueOnce(first.connection)
      .mockResolvedValue(second.connection);
    await client.open();
    // Only the replacement's phases matter below; the first open already
    // announced its own "ready".
    (callbacks.onConnectionState as jest.Mock).mockClear();

    // A token refresh is already in flight (the token endpoint is slow), and a
    // replacement connection is being built meanwhile.
    let refuseToken!: (reason: unknown) => void;
    (refreshCredentials as jest.Mock).mockReturnValue(
      new Promise((_resolve, reject) => { refuseToken = reject; }),
    );
    // Scoped: `classifyNatsError` is a module-level mock shared with every
    // other describe in this file, so the override is undone below rather than
    // left for whichever suite runs next.
    jest.mocked(classifyNatsError).mockReturnValue("expired");
    try {
      const firstStatus = (first.connection as unknown as { status(): Messages<never> }).status();
      firstStatus.push({ type: "error", error: new Error("token expired") } as never);
      await tick();

      const flushed = deferred<void>();
      (second.connection.flush as jest.Mock).mockReturnValue(flushed.promise);
      const replacement = client.open();
      await tick();

      // The desktop refuses the rotation: this device is revoked. The revoked
      // effect disposes the *serving* generation, so the half-built replacement
      // is still alive and reaches its readiness check.
      refuseToken(new RemoteApiError("device revoked", "credentials_revoked", 401));
      await tick();
      flushed.resolve();
      await replacement;

      // Publishing a candidate into a revoked client would hand the UI a socket
      // the pairing may no longer use, so it is closed without ever being
      // announced as ready.
      expect(second.connection.close).toHaveBeenCalled();
      expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("revoked");
      expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
    } finally {
      jest.mocked(classifyNatsError).mockImplementation(() => "transport");
    }
  });

  test("a reconnect frame already in flight when a newer generation took over is dropped", async () => {
    const first = socket();
    const second = socket();
    (wsconnect as jest.Mock)
      .mockResolvedValueOnce(first.connection)
      .mockResolvedValue(second.connection);
    await client.open();

    // The retired socket's status stream delivers a reconnect, and its
    // handshake is paused. While it is paused a newer generation takes over.
    const stale = deferred<unknown>();
    jest.spyOn(client as never, "performHandshake")
      .mockReturnValueOnce(stale.promise as never)
      .mockResolvedValue(confirmation as never);
    const firstStatus = (first.connection as unknown as { status(): Messages<never> }).status();
    firstStatus.push({ type: "reconnect" } as never);
    await tick();

    const replacement = client.open();
    await tick();
    expect(client.accessIdentity).toBe("bridge");

    // The paused frame answers with a *stale* bridge identity. Applying it would
    // rebind the live connection to a process the desktop has already replaced
    // \u2014 every later command would be addressed to a dead bridge instance.
    stale.resolve({
      bridgeInstanceId: "stale-bridge",
      presence: { bridgeInstanceId: "stale-bridge", online: true },
      features: [],
    } as never);
    await tick();
    await replacement;

    expect(client.accessIdentity).toBe("bridge");
    expect(callbacks.onError).not.toHaveBeenCalled();
    expect(jest.mocked(callbacks.onReconnected).mock.calls.length).toBe(2);
  });
});

describe("transfer chunks", () => {
  let client: RemoteClient;
  let callbacks: RemoteClientCallbacks;

  beforeEach(() => {
    jest.useFakeTimers();
    jest.clearAllMocks();
    jest.spyOn(console, "warn").mockImplementation(() => {});
    (ensureFreshCredentials as jest.Mock).mockResolvedValue(credentials);
    callbacks = {
      onConnectionState: jest.fn(),
      onCredentials: jest.fn(async () => {}),
      onError: jest.fn(),
      onEvent: jest.fn(),
      onEventDecodeFailure: jest.fn(),
      onFeatures: jest.fn(),
      onPresence: jest.fn(),
      onReconnected: jest.fn(),
      onSessions: jest.fn(),
      onWorkspaces: jest.fn(),
    };
    client = new RemoteClient(credentials, callbacks);
    jest.spyOn(client as never, "performHandshake").mockResolvedValue({
      bridgeInstanceId: "bridge",
      presence: { bridgeInstanceId: "bridge", online: true },
    } as never);
    jest.spyOn(client as never, "activateSecureChannel").mockResolvedValue(undefined as never);
    const boundary = client as unknown as {
      secureChannels: WeakMap<object, unknown>;
      secureRequest: (connection: NatsConnection, subject: string, bytes: Uint8Array, timeout: number) => Promise<Msg>;
    };
    jest.spyOn(boundary.secureChannels, "get").mockReturnValue({
      open: (_: string, bytes: Uint8Array) => bytes,
      destroy: () => {},
    });
    boundary.secureRequest = (connection, subject, bytes, timeout) =>
      connection.request(subject, bytes, { timeout }) as Promise<Msg>;
  });

  afterEach(async () => {
    await client.close();
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  /** An opened client whose desktop reports itself online and healthy. */
  async function ready() {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    (client as unknown as { receivePresence(p: unknown): void })
      .receivePresence({ agentAvailable: true, bridgeInstanceId: "bridge", online: true });
    return live;
  }
  /** Answer every command request with the given JSON RPC envelope. */
  function reply(live: ReturnType<typeof socket>, body: unknown) {
    jest.mocked(live.connection.request).mockImplementation(async () =>
      ({ data: new TextEncoder().encode(JSON.stringify(body)) }) as never);
  }
  const downloads = () => (client as unknown as { downloadWaiters: Map<string, unknown> }).downloadWaiters;

  test("an upload chunk reports the desktop's refusal instead of the raw envelope", async () => {
    const live = await ready();
    reply(live, { data: {}, error: "transfer_quota_exceeded", success: false });
    await expect(client.uploadChunk("t1", 0, new Uint8Array([1, 2, 3])))
      .rejects.toThrow("transfer_quota_exceeded");
    expect(live.connection.request).toHaveBeenCalledWith(
      "p.pair.xfer.up.t1.chunk.0", expect.any(Uint8Array), { timeout: 15_000 });
  });

  test("an upload chunk with no error text still fails loudly", async () => {
    const live = await ready();
    reply(live, { data: {}, success: false });
    await expect(client.uploadChunk("t1", 0, new Uint8Array())).rejects.toThrow("upload_chunk_failed");
  });

  test("transfers refuse to run before the desktop is business-ready", async () => {
    // Never opened: the transport exists in tests but the client is unpaired,
    // and a chunk sent now would hang on a peer that cannot serve it.
    await expect(client.uploadChunk("t1", 0, new Uint8Array())).rejects.toThrow("communication_frozen");
    await expect(client.downloadChunk("t1", 0)).rejects.toThrow("communication_frozen");
  });

  test("a downloaded chunk is delivered by its own pull subscription", async () => {
    const live = await ready();
    reply(live, { data: {}, success: true });
    const pull = client.downloadChunk("t1", 3);
    await tick();
    live.subscriptions.get("p.pair.xfer.down.>")!
      .push({ subject: "p.pair.xfer.down.t1.chunk.3", data: new Uint8Array([9, 8, 7]) } as Msg);
    await expect(pull).resolves.toEqual(new Uint8Array([9, 8, 7]));
    expect(downloads().size).toBe(0);
  });

  test.each([
    // Not three parts: cannot be attributed to a waiter.
    "p.pair.xfer.down.t1.extra.chunk.3",
    // A non-chunk part of the transfer subject is not chunk data.
    "p.pair.xfer.down.t1.status",
  ])("a transfer message on %s cannot resolve a pull", async subject => {
    const live = await ready();
    reply(live, { data: {}, success: true });
    const pull = client.downloadChunk("t1", 3);
    await tick();
    live.subscriptions.get("p.pair.xfer.down.>")!
      .push({ subject, data: new Uint8Array([1]) } as Msg);
    await tick();
    // Still pending: proven by the timeout below, not by a bare assertion.
    jest.advanceTimersByTime(15_000);
    await expect(pull).rejects.toThrow("download_chunk_timeout");
  });

  test("a chunk that never arrives times out and frees its waiter", async () => {
    const live = await ready();
    reply(live, { data: {}, success: true });
    const pull = client.downloadChunk("t1", 0);
    await tick();
    jest.advanceTimersByTime(15_000);
    await expect(pull).rejects.toThrow("download_chunk_timeout");
    // No leak: an abandoned waiter must not be silently satisfied by a later
    // chunk at the same index.
    expect(downloads().size).toBe(0);
  });

  test("a refused pull rejects before any chunk arrives", async () => {
    const live = await ready();
    reply(live, { data: {}, error: "transfer_unknown", success: false });
    await expect(client.downloadChunk("t1", 0)).rejects.toThrow("transfer_unknown");
    expect(downloads().size).toBe(0);
  });

  test("losing the connection rejects a chunk pull instead of leaving it hanging", async () => {
    const live = await ready();
    // The pull ACK arrives; the binary chunk never does.
    reply(live, { data: {}, success: true });
    const pull = client.downloadChunk("t1", 0);
    await tick();
    // The desktop goes away mid-transfer; the awaiting puller must be told now,
    // not left until the 15 s timer or forever.
    await client.close();
    await expect(pull).rejects.toThrow(/clos/i);
    expect(live.close).toHaveBeenCalled();
  });
});

describe("command retry policy", () => {
  let client: RemoteClient;
  let callbacks: RemoteClientCallbacks;

  beforeEach(() => {
    jest.useFakeTimers();
    jest.clearAllMocks();
    jest.spyOn(console, "warn").mockImplementation(() => {});
    (ensureFreshCredentials as jest.Mock).mockResolvedValue(credentials);
    callbacks = {
      onConnectionState: jest.fn(),
      onCredentials: jest.fn(async () => {}),
      onError: jest.fn(),
      onEvent: jest.fn(),
      onEventDecodeFailure: jest.fn(),
      onFeatures: jest.fn(),
      onPresence: jest.fn(),
      onReconnected: jest.fn(),
      onSessions: jest.fn(),
      onWorkspaces: jest.fn(),
    };
    client = new RemoteClient(credentials, callbacks);
    jest.spyOn(client as never, "performHandshake").mockResolvedValue({
      bridgeInstanceId: "bridge",
      presence: { bridgeInstanceId: "bridge", online: true },
    } as never);
    jest.spyOn(client as never, "activateSecureChannel").mockResolvedValue(undefined as never);
    const boundary = client as unknown as {
      secureChannels: WeakMap<object, unknown>;
      secureRequest: (connection: NatsConnection, subject: string, bytes: Uint8Array, timeout: number) => Promise<Msg>;
    };
    jest.spyOn(boundary.secureChannels, "get").mockReturnValue({
      open: (_: string, bytes: Uint8Array) => bytes,
      destroy: () => {},
    });
    boundary.secureRequest = (connection, subject, bytes, timeout) =>
      connection.request(subject, bytes, { timeout }) as Promise<Msg>;
  });

  afterEach(async () => {
    await client.close();
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  async function ready() {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    (client as unknown as { receivePresence(p: unknown): void })
      .receivePresence({ agentAvailable: true, bridgeInstanceId: "bridge", online: true });
    return live;
  }

  test.each(["get_session_entries", "list_skills", "upload_complete"])(
    "a read-like command (%s) is retried and the half-open transport is revalidated first",
    async type => {
      const live = await ready();
      // classifyNatsError is mocked to "transport" for this suite, so a plain
      // Error is a transient failure 閳?the class the retry loop exists for.
      jest.mocked(live.connection.request)
        .mockRejectedValueOnce(new Error("request timeout"))
        .mockResolvedValueOnce({ data: new TextEncoder().encode(JSON.stringify({ data: { ok: true }, success: true })) } as never);
      const recover = jest.spyOn(client, "recoverAfterTransientRequest").mockResolvedValue(true);
      const pending = client.requestRetry({ type }, "session");
      await jest.advanceTimersByTimeAsync(250);
      await expect(pending).resolves.toMatchObject({ success: true });
      expect(live.connection.request).toHaveBeenCalledTimes(2);
      // The retry is spent only after the transport was validated/rebuilt.
      expect(recover).toHaveBeenCalledTimes(1);
    },
  );

  test("a write-like command that is not receipt-backed gets exactly one attempt", async () => {
    const live = await ready();
    jest.mocked(live.connection.request).mockRejectedValue(new Error("request timeout"));
    await expect(client.requestRetry({ type: "delete_session" }, "session"))
      .rejects.toThrow("request timeout");
    // Deleting twice is not idempotent without a receipt contract, so a retry
    // would risk a second destructive operation.
    expect(live.connection.request).toHaveBeenCalledTimes(1);
  });

  test("a business rejection is never retried, however read-like the command", async () => {
    const live = await ready();
    jest.mocked(live.connection.request).mockResolvedValue({
      data: new TextEncoder().encode(JSON.stringify({
        data: {}, error: "permission_denied", success: false,
      })),
    } as never);
    await expect(client.requestRetry({ type: "list_skills" }, "session"))
      .rejects.toThrow("permission_denied");
    // A refusal is an answer, not a transport failure: retrying it would only
    // repeat the same refusal three times.
    expect(live.connection.request).toHaveBeenCalledTimes(1);
  });

  test("a repeated transient failure exhausts the attempts and rethrows the last error", async () => {
    const live = await ready();
    jest.mocked(live.connection.request).mockRejectedValue(new Error("request timeout"));
    const recover = jest.spyOn(client, "recoverAfterTransientRequest").mockResolvedValue(true);
    const pending = client.requestRetry({ type: "list_skills" }, "session");
    const assertion = expect(pending).rejects.toThrow("request timeout");
    await jest.advanceTimersByTimeAsync(250 + 500);
    await assertion;
    expect(live.connection.request).toHaveBeenCalledTimes(3);
    expect(recover).toHaveBeenCalledTimes(2);
  });

  test("a command recovers only for a transport failure", async () => {
    const recover = jest.spyOn(client, "recoverNow").mockResolvedValue(undefined);
    // A business reply error is not a reason to rebuild the socket.
    const business = new Error("command_failed");
    Object.assign(business, { name: "RemoteResponseError" });
    await expect(client.recoverAfterTransientRequest(business)).resolves.toBe(true);
    recover.mockClear();
    await expect(client.recoverAfterTransientRequest(new Error("not_connected"))).resolves.toBe(true);
    expect(recover).toHaveBeenCalledWith("request-failure");
  });

  test("a recovery that itself fails still reports the request as transient", async () => {
    jest.spyOn(client, "recoverNow").mockRejectedValue(new Error("recovery exploded"));
    // Best-effort by contract: the bounded retry owns the error the caller sees.
    await expect(client.recoverAfterTransientRequest(new Error("request timeout"))).resolves.toBe(true);
  });
});

/**
 * The broker's own status stream is the transport-truth feeder. Every branch of
 * it decides whether the phone keeps, replaces, or gives up on a connection, so
 * each is asserted through the callbacks the UI actually reads.
 */
describe("broker status stream", () => {
  type StatusEvent = { type: string; error?: Error };
  let client: RemoteClient;
  let callbacks: RemoteClientCallbacks;
  let status: Messages<never>;
  const confirmation = {
    bridgeInstanceId: "bridge",
    presence: { bridgeInstanceId: "bridge", online: true },
    features: [],
  };

  /** A socket whose status stream this suite can drive. */
  function liveSocket() {
    const created = socket();
    status = (created.connection as unknown as { status: () => Messages<never> }).status();
    return created;
  }
  const emit = async (event: StatusEvent) => {
    status.push(event as never);
    await tick();
  };

  beforeEach(async () => {
    jest.useFakeTimers();
    jest.clearAllMocks();
    jest.mocked(classifyNatsError).mockReturnValue("transport");
    jest.mocked(jwtExpiry).mockReturnValue(Math.floor(Date.now() / 1000) + 3600);
    jest.spyOn(console, "warn").mockImplementation(() => {});
    (ensureFreshCredentials as jest.Mock).mockResolvedValue(credentials);
    (refreshCredentials as jest.Mock).mockResolvedValue(credentials);
    callbacks = {
      onCredentials: jest.fn(async () => {}),
      onEvent: jest.fn(),
      onEventDecodeFailure: jest.fn(),
      onPresence: jest.fn(),
      onSessions: jest.fn(),
      onWorkspaces: jest.fn(),
      onFeatures: jest.fn(),
      onConnectionState: jest.fn(),
      onReconnected: jest.fn(),
      onError: jest.fn(),
      onCatalogEpoch: jest.fn(),
    };
    client = new RemoteClient(credentials, callbacks);
    jest.spyOn(client as never, "performHandshake").mockResolvedValue(confirmation as never);
    jest.spyOn(client as never, "activateSecureChannel").mockResolvedValue(undefined as never);
    const boundary = client as unknown as {
      secureChannels: WeakMap<object, unknown>;
      secureRequest: (c: NatsConnection, s: string, b: Uint8Array, t: number) => Promise<Msg>;
    };
    jest.spyOn(boundary.secureChannels, "get").mockReturnValue({ open: (_: string, bytes: Uint8Array) => bytes, destroy: () => {} });
    boundary.secureRequest = (connection, subject, bytes, timeout) => connection.request(subject, bytes, { timeout });
  });
  afterEach(async () => {
    await client.close();
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  test("a broker disconnect disposes the generation and reopens immediately", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    const sockets = jest.mocked(wsconnect).mock.calls.length;

    await emit({ type: "disconnect" });

    // The user must not be left on a dead socket while the bounded retry waits:
    // a disconnect builds the replacement now, and the recovery deadline (not
    // the retry ladder) owns how long that may take.
    expect(jest.mocked(wsconnect).mock.calls.length).toBe(sockets + 1);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    expect(callbacks.onError).not.toHaveBeenCalled();
  });

  test("an authorization failure on the status stream is recorded and ends the loop", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    (refreshCredentials as jest.Mock).mockClear();
    jest.mocked(classifyNatsError).mockReturnValue("authorization");

    await emit({ type: "error", error: new Error("authorization violation") });

    // The refusal is recorded with its own support category (AU001) so a
    // support report can tell it apart from a network or protocol failure, and
    // the one available recovery 閳?rotating the token 閳?is attempted exactly
    // once rather than queued per status frame.
    const warnings = jest.mocked(console.warn).mock.calls.map(call => call[1]);
    expect(warnings).toContainEqual(expect.objectContaining({ category: "service_authorization" }));
    expect(warnings).toContainEqual(expect.objectContaining({ supportCode: "AU001" }));
    expect(refreshCredentials).toHaveBeenCalledTimes(1);
  });

  test("a protocol failure on the status stream retires the generation", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    jest.mocked(callbacks.onError).mockClear();
    jest.mocked(classifyNatsError).mockReturnValue("protocol");

    await emit({ type: "error", error: new Error("protocol error") });

    // A malformed frame is not a credential problem: no rotation is spent, and
    // the report carries the protocol support category (PT001).
    expect(refreshCredentials).not.toHaveBeenCalled();
    const warnings = jest.mocked(console.warn).mock.calls.map(call => call[1]);
    expect(warnings).toContainEqual(expect.objectContaining({ category: "protocol" }));
  });

  test("a revoked pairing is not rotated again by the refresh timer", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    jest.mocked(refreshCredentials).mockClear();
    // Control: the timer armed by the ready state does rotate the token on its
    // own schedule, so the silence below is about the terminal state and not a
    // dud timer.
    await jest.advanceTimersByTimeAsync(3_600_000);
    expect(refreshCredentials).toHaveBeenCalled();
    jest.mocked(refreshCredentials).mockClear();
    // The desktop revokes this device, so the client is terminal from here on.
    jest.mocked(ensureFreshCredentials).mockRejectedValue(
      new RemoteApiError("revoked", "credentials_revoked", 401),
    );
    client.setAppActive(false);
    client.setAppActive(true);
    await client.recoverNow("foreground");
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("revoked");
    // A later refresh tick must not spend a rotation on credentials the desktop
    // has already refused.
    await jest.advanceTimersByTimeAsync(7_200_000);
    expect(refreshCredentials).not.toHaveBeenCalled();
  });

  test("a broker reconnect re-handshakes and re-announces readiness", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    jest.mocked(callbacks.onReconnected).mockClear();
    jest.mocked(callbacks.onCatalogEpoch as jest.Mock).mockClear();

    await emit({ type: "reconnect" });

    // Reconnected without rebuilding the socket: the handshake barrier runs
    // again and the UI is told, with the fresh catalog epoch and features.
    expect(callbacks.onReconnected).toHaveBeenCalledTimes(1);
    expect(callbacks.onCatalogEpoch).toHaveBeenCalledWith(undefined);
    expect(callbacks.onFeatures).toHaveBeenCalledWith([]);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });

  test("a reconnect whose handshake fails retires that generation instead of claiming ready", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    jest.spyOn(client as never, "ensureHandshake").mockRejectedValueOnce(new Error("handshake refused") as never);
    jest.mocked(callbacks.onConnectionState).mockClear();

    await emit({ type: "reconnect" });

    // The generation that failed its readiness barrier must not be advertised
    // as usable: no `ready` may be emitted for it.
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("a presence callback that tears the client down during handshaking cancels the ready announcement", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    const sockets = jest.mocked(wsconnect).mock.calls.length;
    (callbacks.onConnectionState as jest.Mock).mockClear();
    (callbacks.onReconnected as jest.Mock).mockClear();
    // The desktop unpairs while the broker is reconnecting. The app's own
    // `onPresence` handler closes the client synchronously (that is exactly
    // what useRemoteConnection:278 does), and the client hands presence to that
    // handler *before* it announces readiness on the same code path.
    (callbacks.onPresence as jest.Mock).mockImplementationOnce(() => { void client.close(); });
    jest.spyOn(client as never, "performHandshake").mockResolvedValue({
      bridgeInstanceId: "bridge",
      presence: { bridgeInstanceId: "bridge", online: true, unpaired: true },
      features: [],
    } as never);

    await emit({ type: "reconnect" });

    // The callback really ran, so the assertions below are about the state the
    // reconnect path leaves behind, not about a reconnect that never started.
    expect(callbacks.onPresence).toHaveBeenCalled();
    // A client the app has already closed must not be announced as ready, and
    // must not build another socket behind the user's back.
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
    expect(jest.mocked(wsconnect).mock.calls.length).toBe(sockets);
    // NOTE for reviewers: this scenario executes `signal`'s ready guard
    // (client.ts:804) but does not *discriminate* it — see module-mobile §8,
    // "covered but inert". `onReconnected` still fires at 1049, which is the
    // pre-existing inconsistency already reported in segment 4, finding 2.
  });

  test("a status stream that ends on a live generation reports the connection as exhausted", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    const sockets = jest.mocked(wsconnect).mock.calls.length;

    status.end();
    await tick();

    // A silently ended iterator is a dead feeder: it must fall back to the same
    // failure path, not leave the phone believing it is connected forever.
    expect(callbacks.onError).toHaveBeenCalled();
    expect(jest.mocked(wsconnect).mock.calls.length).toBeGreaterThanOrEqual(sockets);
  });

  test("an expired-JWT broker error rotates the token instead of giving up", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    (refreshCredentials as jest.Mock).mockClear();
    jest.mocked(classifyNatsError).mockReturnValue("expired");

    await emit({ type: "error", error: new Error("user authentication expired") });

    // One rotation, and the fresh credentials are persisted before the reopen.
    expect(refreshCredentials).toHaveBeenCalledTimes(1);
    expect(callbacks.onCredentials).toHaveBeenCalled();
    expect(callbacks.onError).not.toHaveBeenCalled();
  });

  test("a revoked refresh token stops the client and asks the user to re-pair", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    (refreshCredentials as jest.Mock).mockRejectedValueOnce(
      new RemoteApiError("revoked", "credentials_revoked", 401),
    );
    jest.mocked(classifyNatsError).mockReturnValue("expired");

    await emit({ type: "error", error: new Error("user authentication expired") });
    await tick();

    // Terminal: no retry timer may keep a revoked device dialling.
    expect(callbacks.onError).toHaveBeenCalled();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("revoked");
  });

  test("a retryable refresh failure backs off instead of spinning on the old token", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    (refreshCredentials as jest.Mock).mockRejectedValueOnce(new Error("token endpoint 503"));
    jest.mocked(classifyNatsError).mockReturnValue("expired");

    await emit({ type: "error", error: new Error("user authentication expired") });
    await tick();

    // The caller is told, and the bounded retry (not an immediate second
    // refresh) owns the next attempt.
    expect(callbacks.onError).toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenLastCalledWith("revoked");
    expect(refreshCredentials).toHaveBeenCalledTimes(1);
  });

  test("a malformed JWT is reported and drops the connection instead of looping", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    jest.mocked(callbacks.onError).mockClear();
    // The next scheduling round sees a JWT it cannot read an expiry from. The
    // refresh timer armed at `ready` fires 60s before the (then valid) expiry.
    jest.mocked(jwtExpiry).mockReturnValue(null);

    await jest.advanceTimersByTimeAsync(4_000_000);

    expect(callbacks.onError).toHaveBeenCalledWith(new Error("invalid_jwt"));
    // A token that cannot be decoded is not retried: the client is terminal, so
    // it cannot sit in a 5s invalid-JWT loop.
    await expect(client.request({ type: "list_sessions" })).rejects.toThrow("communication_frozen");
  });

  test("a token rotation that does not help turns into a terminal service failure", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    // The rotation itself keeps failing (the token endpoint rejects), so the
    // auth counter never resets, and every further attempt is also refused by
    // the broker.
    (refreshCredentials as jest.Mock).mockRejectedValue(new Error("token endpoint 503"));
    jest.mocked(classifyNatsError).mockReturnValue("authorization");
    (wsconnect as jest.Mock).mockRejectedValue(new Error("authorization violation"));
    jest.mocked(callbacks.onError).mockClear();

    // First refusal: rotate the token once.
    await client.open();
    expect(refreshCredentials).toHaveBeenCalledTimes(1);

    // Second refusal after a refresh that did not help: a freshly issued token
    // the broker still rejects proves this is not expiry, so the client stops
    // instead of spinning at one full RTT per cycle with no backoff.
    await client.open();
    expect(refreshCredentials).toHaveBeenCalledTimes(1);
    expect(callbacks.onError).toHaveBeenCalledWith(
      expect.objectContaining({ message: expect.stringContaining("remote_service_misconfigured") }),
    );
  });

  test("a malformed event frame is reported instead of taking the connection down", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    const events = live.subscriptions.get("p.pair.evt.>");
    expect(events).toBeDefined();

    events!.push({ subject: "p.pair.evt.s1", data: encoder.encode("not json") } as never);
    await tick();

    // One bad frame must not be treated as a transport fault: the caller is
    // told which session failed, and the connection stays usable.
    expect(callbacks.onEventDecodeFailure).toHaveBeenCalledWith(
      "s1",
      expect.objectContaining({ message: expect.stringContaining("remote_event_decode_failed") }),
    );
    expect(callbacks.onEvent).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("a session and a workspace snapshot reach the catalogue callbacks", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    const session = { sessionId: "s1", threadId: "t1", title: "One", streaming: false };
    const version = { epoch: "e", revision: 3 };

    live.subscriptions.get("p.pair.state.>")!.push({
      subject: "p.pair.state.sessions",
      data: encoder.encode(JSON.stringify({ sessions: [session], version })),
    } as never);
    await tick();
    live.subscriptions.get("p.pair.state.>")!.push({
      subject: "p.pair.state.workspaces",
      data: encoder.encode(JSON.stringify({ workspaces: [{ id: "w1", name: "W" }], version })),
    } as never);
    await tick();

    expect(callbacks.onSessions).toHaveBeenCalledWith([session], version);
    expect(callbacks.onWorkspaces).toHaveBeenCalledWith([{ id: "w1", name: "W" }], version);
  });

  test("a snapshot with no payload reports an empty list, never undefined", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();

    live.subscriptions.get("p.pair.state.>")!.push({
      subject: "p.pair.state.sessions",
      data: encoder.encode("{}"),
    } as never);
    await tick();

    // The catalogue must clear itself from a valid-but-empty snapshot rather
    // than hand `undefined` to a setState that would then throw.
    expect(callbacks.onSessions).toHaveBeenCalledWith([], undefined);
  });

  test("a presence frame from another bridge forces a fresh handshake", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    const handshake = jest.spyOn(client as never, "performHandshake");
    handshake.mockClear();
    jest.mocked(callbacks.onReconnected).mockClear();
    jest.mocked(callbacks.onPresence).mockClear();

    live.subscriptions.get("p.pair.presence")!.push({
      subject: "p.pair.presence",
      data: encoder.encode(JSON.stringify({ bridgeInstanceId: "someone-else", online: true })),
    } as never);
    await tick();

    // A bridge id the phone never confirmed means the desktop restarted (or a
    // different desktop is answering): the identity has to be re-established
    // before any of that peer's state is trusted.
    expect(handshake).toHaveBeenCalled();
    expect(callbacks.onReconnected).toHaveBeenCalled();
    expect(callbacks.onPresence).toHaveBeenCalled();
  });

  test("a presence frame with an unreadable payload is ignored, not fatal", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    const handshake = jest.spyOn(client as never, "performHandshake");
    handshake.mockClear();
    jest.mocked(callbacks.onPresence).mockClear();

    live.subscriptions.get("p.pair.presence")!.push({
      subject: "p.pair.presence",
      data: encoder.encode("<html>proxy</html>"),
    } as never);
    await tick();

    expect(callbacks.onPresence).not.toHaveBeenCalled();
    expect(handshake).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("rotated credentials are persisted before the socket is used", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    const rotated = { ...credentials, userJwt: "jwt-2" } as RemoteCredentials;
    (ensureFreshCredentials as jest.Mock).mockResolvedValue(rotated);
    jest.mocked(callbacks.onCredentials).mockClear();

    await client.open();

    // A token the endpoint refreshed during `open` has to be written down
    // before it is used, or a crash mid-handshake would resurrect the old one.
    expect(callbacks.onCredentials).toHaveBeenCalledWith(rotated);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("a revoked device is reported as revoked and stops dialling", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    jest.mocked(callbacks.onError).mockClear();
    // The next attempt is refused with the credential code the server uses for
    // a device the user unpaired on the desktop.
    (ensureFreshCredentials as jest.Mock).mockRejectedValue(
      new RemoteApiError("revoked", "credentials_revoked", 401),
    );

    await client.open();
    await tick();

    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("revoked");
    expect(callbacks.onError).toHaveBeenCalled();
    const warnings = jest.mocked(console.warn).mock.calls.map(call => call[1]);
    expect(warnings).toContainEqual(
      expect.objectContaining({ category: "credential_revoked", supportCode: "PA001" }),
    );
  });

  test("the confirmed bridge identity is exposed only once a handshake named one", async () => {
    expect(client.accessIdentity).toBeUndefined();
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    expect(client.accessIdentity).toBe("bridge");
  });

  test("a misconfigured service is fatal and stops every timer", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    jest.mocked(callbacks.onError).mockClear();
    // The server refuses the account itself (not the token): retrying cannot
    // ever succeed, so the client has to stop dialling and say so.
    (ensureFreshCredentials as jest.Mock).mockRejectedValue(
      new RemoteApiError("misconfigured", "remote_service_misconfigured", 500),
    );

    await client.open();
    await tick();

    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    expect(callbacks.onError).toHaveBeenCalledWith(
      expect.objectContaining({ message: "misconfigured" }),
    );
    // Terminal: a later network notification must not reopen it.
    const sockets = jest.mocked(wsconnect).mock.calls.length;
    await client.recoverNow("network-restored");
    await tick();
    expect(jest.mocked(wsconnect).mock.calls.length).toBe(sockets);
  });

  test("a socket that outlives its attempt is closed instead of being installed", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    // The handshake is slow enough that the user closes the app while it is in
    // flight: the socket that arrives afterwards belongs to nobody.
    const handshake = deferred<unknown>();
    jest.spyOn(client as never, "performHandshake").mockReturnValue(handshake.promise as never);

    const opening = client.open();
    await tick();
    await client.close();
    handshake.resolve(confirmation);
    await opening;
    await tick();

    expect(live.close).toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("a handshake failure is reported as a desktop handshake problem", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    jest.spyOn(client as never, "performHandshake").mockRejectedValue(new Error("peer mismatch") as never);

    await client.open();

    // The user-facing support string has to name the phase that failed, not
    // only the transport: "could not connect" and "the desktop refused us"
    // need different actions.
    const reported = jest.mocked(callbacks.onError).mock.calls.at(-1)?.[0];
    expect(reported?.message).toContain("desktop_handshake_failed");
    expect(live.close).toHaveBeenCalled();
  });

  test("repeated critical-subscription losses become a single terminal failure", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    jest.mocked(callbacks.onError).mockClear();

    // Every critical subscription of one generation dies in turn. Each on its
    // own is a reconnect (bounded), but the cap on system failures inside the
    // window exists so a peer that keeps dropping channels cannot make the
    // phone reconnect forever: past the cap this is a terminal fault, reported
    // once, with the generation_unhealthy support code (RT001).
    live.subscriptions.get("p.pair.evt.>")!.end();
    await tick();
    live.subscriptions.get("p.pair.presence")!.end();
    await tick();
    live.subscriptions.get("p.pair.state.>")!.end();
    await tick();
    live.subscriptions.get("p.pair.xfer.down.>")!.end();
    await tick();

    const warnings = jest.mocked(console.warn).mock.calls.map(call => call[1]);
    expect(warnings).toContainEqual(
      expect.objectContaining({ category: "generation_unhealthy" }),
    );
    expect(callbacks.onConnectionState).not.toHaveBeenLastCalledWith("ready");
  });

  test("a frame the channel cannot decrypt is dropped without killing the connection", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    const boundary = client as unknown as {
      secureChannels: { get(c: object): { open(s: string, b: Uint8Array): Uint8Array; destroy(): void } | undefined };
    };
    // A forged, replayed, or retired-channel frame arrives: the AEAD refuses it.
    boundary.secureChannels.get = () => ({ open: () => { throw new Error("aead rejected"); }, destroy: () => {} });
    jest.mocked(callbacks.onEvent).mockClear();

    live.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.s1",
      data: encoder.encode(JSON.stringify({ type: "text_chunk", data: "{}" })),
    } as never);
    await tick();

    // No event leaks to the UI, and the generation is not retired for it: one
    // undecryptable frame is not proof that the peer is gone.
    expect(callbacks.onEvent).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("an event subscription that ends on a live generation retires it", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();

    live.subscriptions.get("p.pair.evt.>")!.end();
    await tick();

    // A critical subscription that silently ended means the peer stopped
    // sending: the generation is unhealthy, not silently half-connected.
    const warnings = jest.mocked(console.warn).mock.calls.map(call => call[1]);
    expect(warnings).toContainEqual(
      expect.objectContaining({ category: "generation_unhealthy", supportCode: "RT001" }),
    );
  });

  test("a burst of events is yielded to input instead of monopolising the loop", async () => {
    const live = liveSocket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    const events = live.subscriptions.get("p.pair.evt.>")!;
    const body = encoder.encode(JSON.stringify({ type: "text_chunk", data: "{}" }));
    // A cached replay can hand the phone thousands of frames at once; the loop
    // must yield to the timer queue rather than decrypt them all in one task.
    for (let i = 0; i < 70; i++) {
      events.push({ subject: `p.pair.evt.s${i}`, data: body } as never);
      await tick();
    }
    // The first wave is delivered before the yield; the loop then pauses for
    // input rather than decrypting the rest in the same task.
    expect(jest.mocked(callbacks.onEvent).mock.calls.length).toBe(64);
    let delivered = jest.mocked(callbacks.onEvent).mock.calls.length;
    for (let round = 0; round < 40 && delivered < 70; round++) {
      await jest.advanceTimersByTimeAsync(1);
      await tick();
      delivered = jest.mocked(callbacks.onEvent).mock.calls.length;
    }
    // Nothing is lost by the pause: the whole burst still reaches the UI.
    expect(delivered).toBe(70);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("a status frame that lands after its generation was retired is not treated as an outage", async () => {
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    (callbacks.onError as jest.Mock).mockClear();

    // A real socket's `close()` drains asynchronously, so the status stream can
    // still hand over a frame that was already in flight when the generation it
    // belongs to was retired (a background dispose, or a replacement taking
    // over). Handling the frame would report an outage that is really this
    // client's own teardown, and would arm a retry for a connection that is
    // already gone.
    (client as unknown as { activeGeneration: { retire(): void } }).activeGeneration.retire();
    await emit({ type: "reconnect" });

    expect(callbacks.onError).not.toHaveBeenCalled();
    expect(jest.mocked(wsconnect).mock.calls.length).toBe(1);
    expect(callbacks.onConnectionState).not.toHaveBeenLastCalledWith("reconnecting");
  });

  test("a token refused before this pairing ever served traffic fails the client instead of retrying", async () => {
    // The desktop never reports presence, so the client is connected but has
    // never been usable: `everReady` stays false and the connection budget is
    // the short one.
    jest.spyOn(client as never, "performHandshake").mockResolvedValue({
      bridgeInstanceId: "bridge",
      presence: { bridgeInstanceId: "bridge", online: false },
      features: [],
    } as never);
    (wsconnect as jest.Mock).mockImplementation(async () => liveSocket().connection);
    await client.open();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");

    // The broker rejects the token the phone is holding, and the token endpoint
    // refuses to rotate it.
    jest.mocked(classifyNatsError).mockReturnValue("expired");
    (refreshCredentials as jest.Mock).mockRejectedValue(
      new RemoteApiError("token refused", "pairing_signature_invalid", 400),
    );
    await emit({ type: "error", error: new Error("token expired") });

    // No desktop has ever served this client, so a token it cannot use is a
    // failed connection rather than a retry ladder with no visible end.
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    expect(callbacks.onError).toHaveBeenCalled();
  });
});

describe("exported failure predicates", () => {
  test("only a local readiness gate is a deferred request error", () => {
    // The connection supervisor keeps a `communication_frozen` request pending
    // (it will succeed once ready), but must not do that for a server rejection
    // that merely happens to say the same words.
    expect(isDeferredRequestError(new Error("communication_frozen"))).toBe(true);
    expect(isDeferredRequestError(new Error("something else"))).toBe(false);
    expect(isDeferredRequestError("communication_frozen")).toBe(false);
    expect(isDeferredRequestError(undefined)).toBe(false);
  });

  test("a frozen server reply is not mistaken for the local readiness gate", async () => {
    jest.useFakeTimers();
    jest.clearAllMocks();
    (ensureFreshCredentials as jest.Mock).mockResolvedValue(credentials);
    const callbacks: RemoteClientCallbacks = {
      onCredentials: jest.fn(async () => {}), onEvent: jest.fn(), onEventDecodeFailure: jest.fn(),
      onPresence: jest.fn(), onSessions: jest.fn(), onWorkspaces: jest.fn(), onFeatures: jest.fn(),
      onConnectionState: jest.fn(), onReconnected: jest.fn(), onError: jest.fn(),
    };
    const bound = new RemoteClient(credentials, callbacks);
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    jest.spyOn(bound as never, "performHandshake").mockResolvedValue({
      bridgeInstanceId: "bridge", presence: { bridgeInstanceId: "bridge", online: true }, features: [],
    } as never);
    jest.spyOn(bound as never, "activateSecureChannel").mockResolvedValue(undefined as never);
    const boundary = bound as unknown as {
      secureChannels: WeakMap<object, unknown>;
      secureRequest: (c: NatsConnection, s: string, b: Uint8Array, t: number) => Promise<Msg>;
    };
    jest.spyOn(boundary.secureChannels, "get").mockReturnValue({ open: (_: string, b: Uint8Array) => b, destroy: () => {} });
    boundary.secureRequest = (connection, subject, bytes, timeout) => connection.request(subject, bytes, { timeout });
    try {
      await bound.open();
      (live.connection as unknown as { request: jest.Mock }).request.mockResolvedValue({
        data: encoder.encode(JSON.stringify({ success: false, error: "communication_frozen" })),
      });
      const rejection = await bound.request({ type: "list_sessions" }).catch((error: unknown) => error);
      // The client's own gate keeps such a request pending; a *server* saying it
      // must not, or the caller would wait for a readiness that already failed.
      expect(isDeferredRequestError(rejection)).toBe(false);
    } finally {
      await bound.close();
      jest.useRealTimers();
    }
  });

  test("a transport failure is the only transient NATS request error", () => {
    jest.mocked(classifyNatsError).mockReturnValue("transport");
    expect(isTransientNatsRequestError(new Error("connection reset"))).toBe(true);
    jest.mocked(classifyNatsError).mockReturnValue("authorization");
    // A credential problem is not retried as if it were a flaky socket.
    expect(isTransientNatsRequestError(new Error("authorization violation"))).toBe(false);
  });

  test("a server URL that cannot be parsed is still labelled, never blank", async () => {
    jest.useFakeTimers();
    jest.clearAllMocks();
    (ensureFreshCredentials as jest.Mock).mockResolvedValue(credentials);
    const callbacks: RemoteClientCallbacks = {
      onCredentials: jest.fn(async () => {}), onEvent: jest.fn(), onEventDecodeFailure: jest.fn(),
      onPresence: jest.fn(), onSessions: jest.fn(), onWorkspaces: jest.fn(), onFeatures: jest.fn(),
      onConnectionState: jest.fn(), onReconnected: jest.fn(), onError: jest.fn(),
    };
    const brokenCredentials = { ...credentials, natsWsUrl: "not a url" } as RemoteCredentials;
    (ensureFreshCredentials as jest.Mock).mockResolvedValue(brokenCredentials);
    const broken = new RemoteClient(brokenCredentials, callbacks);
    (wsconnect as jest.Mock).mockRejectedValue(new Error("dial failed"));
    try {
      await broken.open();
      // A support report has to say *which* server failed; falling back to the
      // raw string (or to nothing) would make the report undiagnosable.
      expect(jest.mocked(callbacks.onError).mock.calls.at(-1)?.[0].message).toContain(
        "nats_connect_failed (invalid server URL)",
      );
    } finally {
      await broken.close();
      jest.useRealTimers();
    }
  });
});

/**
 * Lifecycle guards the happy path never takes: a socket that refuses to close, a
 * recovery budget that runs out while a probe is in flight, a callback that
 * closes the client mid-handoff, a rotation that outlives its pairing. Each is
 * asserted on something the app can observe — a state the UI reads, a socket
 * that must not be dialled, a presence that must not be published.
 */
describe("lifecycle guards around a failing transport", () => {
  let client: RemoteClient;
  let callbacks: RemoteClientCallbacks;
  const confirmation = {
    bridgeInstanceId: "bridge",
    presence: { bridgeInstanceId: "bridge", online: true },
  };

  beforeEach(() => {
    jest.useFakeTimers();
    jest.clearAllMocks();
    jest.spyOn(console, "warn").mockImplementation(() => {});
    jest.mocked(ensureFreshCredentials).mockResolvedValue(credentials);
    jest.mocked(classifyNatsError).mockImplementation(() => "transport");
    jest.mocked(jwtExpiry).mockImplementation(() => Math.floor(Date.now() / 1000) + 3600);
    callbacks = {
      onCredentials: jest.fn(async () => {}),
      onEvent: jest.fn(),
      onEventDecodeFailure: jest.fn(),
      onPresence: jest.fn(),
      onSessions: jest.fn(),
      onWorkspaces: jest.fn(),
      onFeatures: jest.fn(),
      onConnectionState: jest.fn(),
      onReconnected: jest.fn(),
      onError: jest.fn(),
    };
    client = new RemoteClient(credentials, callbacks);
    jest.spyOn(client as never, "performHandshake").mockResolvedValue(confirmation as never);
    jest.spyOn(client as never, "activateSecureChannel").mockResolvedValue(undefined as never);
    const boundary = client as unknown as {
      secureChannels: WeakMap<object, unknown>;
      secureRequest: (connection: NatsConnection, subject: string, bytes: Uint8Array, timeout: number) => Promise<Msg>;
    };
    jest.spyOn(boundary.secureChannels, "get").mockReturnValue({
      open: (_: string, bytes: Uint8Array) => bytes,
      destroy: () => {},
    });
    boundary.secureRequest = (connection, subject, bytes, timeout) =>
      connection.request(subject, bytes, { timeout }) as Promise<Msg>;
  });

  afterEach(async () => {
    await client.close();
    jest.mocked(classifyNatsError).mockImplementation(() => "transport");
    jest.mocked(jwtExpiry).mockImplementation(() => Math.floor(Date.now() / 1000) + 3600);
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  /** A promise whose rejection this test controls (the shared `deferred`
   * helper only exposes the resolve side). */
  function deferredFailure<T>() {
    let reject!: (reason?: unknown) => void;
    const promise = new Promise<T>((_resolve, fail) => { reject = fail; });
    return { promise, reject };
  }

  /** The authenticated-probe reply a desktop that still holds our keys sends. */
  const probeReply = () => ({
    data: encoder.encode(JSON.stringify({ success: true, data: {
      pairId: "pair", bridgeInstanceId: "bridge", online: true, lastHeartbeatTs: 0,
    } })),
  });

  /** An opened client whose desktop is online and whose ready signal has
   * already marked it as ever-ready (so failures back off instead of ending). */
  async function openServingSocket() {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(live.connection);
    await client.open();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    return live;
  }

  /**
   * A *failed replacement* is the only way the shared recovery window is armed
   * while a socket can still serve traffic: the new attempt dials and dies, the
   * probe of the old socket fails, and the backoff timer is armed for later.
   */
  async function armBudgetWhileServing(live: ReturnType<typeof socket>) {
    (wsconnect as jest.Mock).mockRejectedValue(new Error("network down"));
    jest.mocked(live.connection.request).mockRejectedValue(new Error("request timeout"));
    await client.open();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("reconnecting");
  }

  test("a healthy serving socket clears the pending retry instead of dialling it again", async () => {
    const live = await openServingSocket();
    await armBudgetWhileServing(live);
    jest.mocked(live.connection.request).mockResolvedValue(probeReply() as never);
    jest.mocked(callbacks.onReconnected).mockClear();
    await client.recoverNow("foreground");
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    expect(callbacks.onReconnected).toHaveBeenCalledTimes(1);
    // The probe proved the old socket still serves this desktop, so the armed
    // backoff must be cancelled: dialling the same dead endpoint again would
    // tear down a healthy socket for nothing.
    await jest.advanceTimersByTimeAsync(30_000);
    expect(wsconnect).toHaveBeenCalledTimes(2);
  });

  test("a budget that runs out during the probe fails the connection and publishes no presence", async () => {
    const live = await openServingSocket();
    await armBudgetWhileServing(live);
    // The probe is answered, but the phone was suspended long enough that the
    // three-minute window elapsed while the reply was in flight.
    jest.mocked(live.connection.request).mockImplementation(async () => {
      jest.setSystemTime(Date.now() + 181_000);
      return probeReply() as never;
    });
    jest.mocked(callbacks.onPresence).mockClear();
    jest.mocked(callbacks.onReconnected).mockClear();
    await client.recoverNow("foreground");
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    expect(callbacks.onError).toHaveBeenLastCalledWith(new Error("recovery_timeout"));
    // A desktop that answered inside a window that has already closed is not
    // "back": publishing it would re-enable every conversation action on a
    // pairing whose recovery budget is spent.
    expect(callbacks.onPresence).not.toHaveBeenCalled();
    expect(callbacks.onReconnected).not.toHaveBeenCalled();
    // The failure tears down the one owner of the retry timer; a late deadline
    // firing afterwards must not report the same failure a second time.
    jest.mocked(callbacks.onError).mockClear();
    jest.mocked(callbacks.onConnectionState).mockClear();
    await jest.advanceTimersByTimeAsync(240_000);
    expect(callbacks.onError).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalled();
    expect(wsconnect).toHaveBeenCalledTimes(2);
  });

  test("a presence callback that closes the client stops the handoff from publishing ready", async () => {
    const live = await openServingSocket();
    await armBudgetWhileServing(live);
    jest.mocked(live.connection.request).mockResolvedValue(probeReply() as never);
    jest.mocked(callbacks.onReconnected).mockClear();
    jest.mocked(callbacks.onConnectionState).mockClear();
    // The app drops the pairing from inside the presence callback. Only the
    // probe's reply carries a heartbeat stamp; the handoff's own presence is
    // published before the foreground recovery even starts.
    jest.mocked(callbacks.onPresence).mockImplementation((presence) => {
      if ((presence as unknown as { lastHeartbeatTs?: number }).lastHeartbeatTs !== undefined) {
        void client.close();
      }
    });
    await client.recoverNow("foreground");
    expect(callbacks.onPresence).toHaveBeenCalledTimes(2);
    expect(callbacks.onReconnected).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("a foreground request whose window already closed never probes the dead socket", async () => {
    const live = await openServingSocket();
    await armBudgetWhileServing(live);
    const probes = jest.mocked(live.connection.request).mock.calls.length;
    jest.setSystemTime(Date.now() + 181_000);
    await client.recoverNow("foreground");
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    expect(callbacks.onError).toHaveBeenLastCalledWith(new Error("recovery_timeout"));
    // The window is closed: spending a probe (and a three-minute flush budget)
    // on a socket this client has already given up on is pure delay.
    expect(jest.mocked(live.connection.request).mock.calls.length).toBe(probes);
  });

  async function startRotation(live: ReturnType<typeof socket>) {
    const status = (live.connection as unknown as { status(): Messages<never> }).status();
    // classifyNatsError is mocked to "transport" for this suite; this one frame
    // is the broker's own token-expiry signal, which starts a rotation.
    jest.mocked(classifyNatsError).mockReturnValueOnce("expired");
    status.push({ type: "error", error: new Error("token expired") } as never);
    await tick();
    expect(refreshCredentials).toHaveBeenCalled();
  }

  test("a rotation that lands after the client was closed is never adopted", async () => {
    const live = await openServingSocket();
    const rotation = deferred<RemoteCredentials>();
    jest.mocked(refreshCredentials).mockReturnValue(rotation.promise);
    await startRotation(live);
    await client.close();
    // The token endpoint answers after the pairing was dropped. Writing those
    // credentials would persist a pairing the user just deleted.
    rotation.resolve({ ...credentials, userJwt: "rotated" });
    await tick();
    expect(callbacks.onCredentials).not.toHaveBeenCalled();
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });

  test("a rotation whose attempt was cancelled while writing credentials does not dial in the same breath", async () => {
    const live = await openServingSocket();
    const rotation = deferred<RemoteCredentials>();
    jest.mocked(refreshCredentials).mockReturnValue(rotation.promise);
    await startRotation(live);
    // The app is backgrounded and foregrounded while the rotated token is being
    // persisted. That suspends the attempt (the socket is closed, the refresh
    // controller aborted) and the foreground handler owns the reconnect — the
    // rotation must not fight it by dialling the dead endpoint itself.
    jest.mocked(callbacks.onCredentials).mockImplementation(async () => {
      client.setAppActive(false);
      client.setAppActive(true);
    });
    rotation.resolve({ ...credentials, userJwt: "rotated" });
    await tick();
    expect(callbacks.onCredentials).toHaveBeenCalledTimes(1);
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });

  test("a rotation that fails on the transport is not reported while the socket still serves", async () => {
    const live = await openServingSocket();
    jest.mocked(refreshCredentials).mockRejectedValue(new Error("token endpoint down"));
    jest.mocked(callbacks.onReconnected).mockClear();
    await startRotation(live);
    await tick();
    // The token endpoint is down, but the desktop is still answering on the
    // serving socket: a failed rotation is not a failed connection, and the
    // user must not see a failure banner for it.
    expect(callbacks.onError).not.toHaveBeenCalled();
    expect(callbacks.onReconnected).toHaveBeenCalledTimes(1);
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
  });

  test("a rejected rotation about nothing is never surfaced", async () => {
    const live = await openServingSocket();
    const rotation = deferredFailure<RemoteCredentials>();
    jest.mocked(refreshCredentials).mockReturnValue(rotation.promise);
    await startRotation(live);
    await client.close();
    // A revoked-credential answer is terminal in the normal flow; after an
    // explicit close it must stay silent instead of reporting a pairing the
    // user already left.
    rotation.reject(new RemoteApiError("revoked", "credentials_revoked", 401));
    await tick();
    expect(callbacks.onError).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("revoked");
  });

  test("a rotation that fails while the client closes during its own probe stays silent", async () => {
    const live = await openServingSocket();
    jest.mocked(refreshCredentials).mockRejectedValue(new Error("token endpoint down"));
    // The rotation's fallback probe finds the pairing closing under it.
    jest.mocked(live.connection.request).mockImplementation(async () => {
      await client.close();
      throw new Error("request timeout");
    });
    await startRotation(live);
    jest.mocked(callbacks.onError).mockClear();
    await tick();
    expect(callbacks.onError).not.toHaveBeenCalled();
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });

  test("a socket that refuses to close is still dropped when the pairing was revoked mid-handshake", async () => {
    const first = socket();
    const second = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(first.connection).mockResolvedValue(second.connection);
    await client.open();
    jest.mocked(callbacks.onConnectionState).mockClear();
    // A token rotation is in flight while the replacement sits at its readiness
    // barrier.
    const rotation = deferredFailure<RemoteCredentials>();
    jest.mocked(refreshCredentials).mockReturnValue(rotation.promise);
    await startRotation(first);
    const flushed = deferred<void>();
    second.flush.mockReturnValue(flushed.promise);
    const replacement = client.open();
    await tick();
    rotation.reject(new RemoteApiError("device revoked", "credentials_revoked", 401));
    await tick();
    // The OS already reaped the candidate socket, so its teardown rejects too.
    second.close.mockRejectedValue(new Error("socket already gone"));
    flushed.resolve();
    await replacement;
    // A revoked pairing must not be left holding a half-built socket.
    expect(second.close).toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("a handshake failure drops its socket even when the teardown itself fails", async () => {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    jest.spyOn(client as never, "performHandshake")
      .mockRejectedValue(new Error("pairing_signature_invalid") as never);
    live.close.mockRejectedValue(new Error("socket already gone"));
    await client.open();
    expect(live.close).toHaveBeenCalled();
    // The rejections are swallowed, but the failure itself still reaches the UI
    // through the stable handshake label.
    expect(jest.mocked(callbacks.onError).mock.calls.at(-1)?.[0].message)
      .toContain("desktop_handshake_failed");
  });

  test("a cancelled attempt drops its half-built socket even when the teardown fails", async () => {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    const handshake = deferred<unknown>();
    jest.spyOn(client as never, "performHandshake").mockReturnValue(handshake.promise as never);
    const opening = client.open();
    await tick();
    live.close.mockRejectedValue(new Error("socket already gone"));
    jest.mocked(callbacks.onConnectionState).mockClear();
    client.setAppActive(false);
    expect(live.close).toHaveBeenCalled();
    handshake.resolve(confirmation);
    await opening;
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("closing the client drops the serving socket even when its teardown fails", async () => {
    const live = await openServingSocket();
    live.close.mockRejectedValue(new Error("socket already gone"));
    await client.close();
    // A socket that fails to close must still be forgotten: holding it would
    // re-enable commands on a pairing the user left.
    expect(live.close).toHaveBeenCalled();
    expect(client.accessIdentity).toBeUndefined();
    await expect(client.request({ type: "list_sessions" })).rejects.toThrow("communication_frozen");
  });

  test("a candidate that loses its subscription is discarded even when its socket refuses to close", async () => {
    const live = socket();
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    const flushed = deferred<void>();
    live.flush.mockReturnValue(flushed.promise);
    const opening = client.open();
    await tick();
    live.close.mockRejectedValue(new Error("socket already gone"));
    live.subscriptions.get("p.pair.state.>")!.end();
    await tick();
    flushed.resolve();
    await opening;
    expect(live.close).toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("a proven replacement retires the previous socket even when that teardown fails", async () => {
    const first = socket();
    const second = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(first.connection).mockResolvedValueOnce(second.connection);
    await client.open();
    first.close.mockRejectedValue(new Error("socket already gone"));
    await client.open();
    // The old socket is only retired once the new one passed its barrier, and a
    // failed teardown must not leave it subscribed to the broker.
    expect(first.close).toHaveBeenCalled();
    expect(second.connection.close).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    await expect(client.request({ type: "list_sessions" })).resolves.toMatchObject({ success: true });
    // Commands are served by the replacement, not by the socket that refused to
    // go away.
    expect(second.connection.request).toHaveBeenCalled();
  });

  test("a credential adoption that outlives its attempt is not followed by a dial", async () => {
    const fresh = { ...credentials, userJwt: "rotated" };
    jest.mocked(ensureFreshCredentials).mockResolvedValue(fresh);
    // The rotated credentials are handed over, and the app closes the pairing
    // while that write is being persisted.
    jest.mocked(callbacks.onCredentials).mockImplementation(async () => { await client.close(); });
    await client.open();
    // The attempt must end there: no socket may be opened with credentials the
    // app has already dropped.
    expect(wsconnect).not.toHaveBeenCalled();
    expect(client.accessIdentity).toBeUndefined();
  });

  test("a healthy heartbeat that arrives after the window closed cannot revive the client", async () => {
    const first = await openServingSocket();
    const second = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(second.connection);
    // The replacement's handshake reports a desktop with no usable agent, which
    // leaves the recovery window armed even though the socket becomes serving.
    jest.spyOn(client as never, "performHandshake").mockResolvedValue({
      bridgeInstanceId: "bridge",
      presence: { bridgeInstanceId: "bridge", online: true, agentAvailable: false },
    } as never);
    await client.open();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("ready");
    expect(first.close).toHaveBeenCalled();
    // The desktop's next heartbeat lands after the window closed.
    jest.setSystemTime(Date.now() + 181_000);
    jest.mocked(callbacks.onPresence).mockClear();
    second.subscriptions.get("p.pair.presence")!.push({
      subject: "p.pair.presence",
      data: encoder.encode(JSON.stringify({ bridgeInstanceId: "bridge", online: true })),
    } as never);
    await tick();
    // A heartbeat inside a window that has already closed is not "back":
    // publishing it would re-enable every conversation action on a pairing whose
    // recovery budget is spent.
    expect(callbacks.onPresence).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    expect(callbacks.onError).toHaveBeenLastCalledWith(new Error("recovery_timeout"));
  });

  test("a feature announcement that closes the client leaves the replacement unpublished", async () => {
    const first = socket();
    const second = socket();
    (wsconnect as jest.Mock).mockResolvedValueOnce(first.connection).mockResolvedValueOnce(second.connection);
    await client.open();
    jest.mocked(callbacks.onReconnected).mockClear();
    jest.mocked(callbacks.onConnectionState).mockClear();
    // The app unpairs from inside the capability announcement of a handoff.
    jest.mocked(callbacks.onFeatures).mockImplementation(() => { void client.close(); });
    await client.open();
    expect(second.connection.close).toHaveBeenCalled();
    expect(callbacks.onReconnected).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("an expired token resets the single rotation allowance instead of failing the service", async () => {
    const live = await openServingSocket();
    // Every dial from here on reaches a desktop that refuses the handshake with
    // an auth error — the refreshable class, not a dead socket.
    (wsconnect as jest.Mock).mockResolvedValue(socket().connection);
    jest.spyOn(client as never, "performHandshake")
      .mockRejectedValue(new Error("pairing_signature_invalid") as never);
    jest.mocked(refreshCredentials).mockRejectedValue(new Error("token endpoint down"));
    // The serving socket is gone as well, so the failed rotation falls back to
    // the bounded retry instead of being papered over by a successful probe.
    jest.mocked(live.connection.request).mockRejectedValue(new Error("request timeout"));
    await client.recoverNow("presence-stale");
    await tick();
    expect(callbacks.onConnectionState).toHaveBeenCalledWith("refreshing");
    expect(refreshCredentials).toHaveBeenCalledTimes(1);
    // The rotated JWT is itself already expired when the second auth failure
    // lands: that is a fresh expiry, not a service misconfiguration, so the
    // single-rotation allowance must reset and rotate again.
    jest.mocked(jwtExpiry).mockReturnValue(0);
    await jest.advanceTimersByTimeAsync(2_000);
    expect(refreshCredentials).toHaveBeenCalledTimes(2);
    expect(jest.mocked(callbacks.onError).mock.calls.map(call => call[0].message).join("|"))
      .not.toContain("remote_service_misconfigured");
  });

  test("a failure that lands after the window closed fails at once instead of arming a retry", async () => {
    const live = await openServingSocket();
    await armBudgetWhileServing(live);
    jest.setSystemTime(Date.now() + 181_000);
    jest.mocked(callbacks.onConnectionState).mockClear();
    // A projection error arrives from the still-attached socket while the
    // recovery window has already run out.
    jest.mocked(callbacks.onEvent).mockImplementation(() => { throw new Error("projection exploded"); });
    live.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.s1", data: encoder.encode('{"type":"agent_end","data":"{}"}'),
    } as never);
    await tick();
    // The budget is spent: the client must fail now, not arm a backoff that
    // would keep dialling a dead endpoint for another three minutes.
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    expect(callbacks.onError).toHaveBeenLastCalledWith(new Error("recovery_timeout"));
    await jest.advanceTimersByTimeAsync(30_000);
    expect(wsconnect).toHaveBeenCalledTimes(2);
  });

  test("backgrounding during the reconnect broadcast opens no failure episode", async () => {
    const live = await openServingSocket();
    jest.mocked(console.warn).mockClear();
    // The app is suspended the moment it is told the transport dropped (a lock
    // screen, a tab switch). It has no retry to own until it comes back.
    jest.mocked(callbacks.onConnectionState).mockImplementation((state) => {
      if (state === "reconnecting") client.setAppActive(false);
    });
    jest.mocked(callbacks.onEvent).mockImplementation(() => { throw new Error("projection exploded"); });
    live.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.s1", data: encoder.encode('{"type":"agent_end","data":"{}"}'),
    } as never);
    await tick();
    // A suspended app must not open a diagnostic failure episode (the bounded
    // support log would record an outage that is not being worked on).
    expect(console.warn).not.toHaveBeenCalled();
    await jest.advanceTimersByTimeAsync(60_000);
    expect(wsconnect).toHaveBeenCalledTimes(1);
  });

  test("a disconnect frame that lands after the window closed fails the connection at once", async () => {
    const live = await openServingSocket();
    await armBudgetWhileServing(live);
    jest.setSystemTime(Date.now() + 181_000);
    jest.mocked(callbacks.onConnectionState).mockClear();
    const status = (live.connection as unknown as { status(): Messages<never> }).status();
    status.push({ type: "disconnect" } as never);
    await tick();
    // The broker dropped the socket, but the recovery window is already spent:
    // the UI must be told the pairing failed, not sent back to "reconnecting"
    // for another three minutes.
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    expect(callbacks.onError).toHaveBeenLastCalledWith(new Error("recovery_timeout"));
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("reconnecting");
    await jest.advanceTimersByTimeAsync(30_000);
    expect(wsconnect).toHaveBeenCalledTimes(2);
  });

  test("a reconnect frame whose window ran out during the handshake is never presented as ready", async () => {
    const live = await openServingSocket();
    await armBudgetWhileServing(live);
    jest.mocked(callbacks.onConnectionState).mockClear();
    // The broker's own reconnect lands, but the budget elapses while the
    // handshake/flush is in flight.
    jest.mocked(live.flush).mockImplementation(async () => {
      jest.setSystemTime(Date.now() + 181_000);
    });
    const status = (live.connection as unknown as { status(): Messages<never> }).status();
    status.push({ type: "reconnect" } as never);
    await tick();
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("failed");
    expect(callbacks.onError).toHaveBeenLastCalledWith(new Error("recovery_timeout"));
    // The re-announcement must not move a client whose window is spent back to
    // ready: every conversation action would be re-enabled on a dead pairing.
    expect(callbacks.onConnectionState).not.toHaveBeenCalledWith("ready");
  });

  test("a late connect attempt on a client that already gave up dials nothing", async () => {
    await client.close();
    // The attempt is ordered by a timer or a callback that was queued before the
    // close took effect; it must not open a socket nobody owns.
    await (client as unknown as { openAttempt(): Promise<void> }).openAttempt();
    expect(wsconnect).not.toHaveBeenCalled();
    expect(callbacks.onConnectionState).not.toHaveBeenCalled();
  });

  test("a failure reported after the client gave up is not re-reported", async () => {
    await openServingSocket();
    await client.close();
    jest.mocked(callbacks.onError).mockClear();
    // A stale timer or a socket callback reports a transport failure after the
    // user closed the pairing: nothing may be surfaced for it.
    (client as unknown as { handleFailure(error: unknown): void })
      .handleFailure(new Error("request timeout"));
    expect(callbacks.onError).not.toHaveBeenCalled();
  });

  test("a deadline that fires after the client was closed reports nothing", async () => {
    await openServingSocket();
    await client.close();
    jest.mocked(callbacks.onError).mockClear();
    // The recovery deadline is cleared on close, but a late callback must not be
    // able to fail a pairing the user has already left.
    (client as unknown as { endUnavailable(error: Error): void })
      .endUnavailable(new Error("recovery_timeout"));
    expect(callbacks.onError).not.toHaveBeenCalled();
    expect(client.accessIdentity).toBeUndefined();
  });

  test("a reconnect frame whose socket closed under it is not re-announced", async () => {
    const live = await openServingSocket();
    const flushed = deferred<void>();
    jest.mocked(live.flush).mockReturnValue(flushed.promise);
    jest.mocked(callbacks.onReconnected).mockClear();
    jest.mocked(callbacks.onFeatures).mockClear();
    const status = (live.connection as unknown as { status(): Messages<never> }).status();
    status.push({ type: "reconnect" } as never);
    await tick();
    // The pairing is dropped while the broker's reconnect is being re-verified.
    await client.close();
    flushed.resolve();
    await tick();
    // A re-announcement on a closed client would re-enable every action on a
    // pairing that is already gone.
    expect(callbacks.onReconnected).not.toHaveBeenCalled();
    expect(callbacks.onFeatures).not.toHaveBeenCalled();
  });

  test("a status stream that throws is reported instead of wedging the client", async () => {
    const live = socket();
    const boom = new Error("status stream broke");
    let reads = 0;
    let failNext: ((error: unknown) => void) | null = null;
    // The broker's status reader works once and then fails (the SDK's reconnect
    // budget was spent and its queue tore down under the loop).
    (live.connection as unknown as { status: () => AsyncIterable<unknown> }).status = () => ({
      [Symbol.asyncIterator]: () => ({
        next: () => (++reads === 1
          ? Promise.resolve({ done: false, value: { type: "update" } })
          : new Promise((_resolve, reject) => { failNext = reject; })),
      }),
    });
    (wsconnect as jest.Mock).mockResolvedValue(live.connection);
    await client.open();
    await tick();
    failNext!(boom);
    await tick();
    expect(jest.mocked(callbacks.onError).mock.calls.map(call => call[0].message).join("|"))
      .toContain("status stream broke");
    // The dead watcher must still leave the client retrying rather than stuck in
    // "connected" with no truth feeder.
    expect(callbacks.onConnectionState).toHaveBeenLastCalledWith("reconnecting");
    await jest.advanceTimersByTimeAsync(2_000);
    expect(wsconnect).toHaveBeenCalledTimes(2);
  });

  test("a transfer chunk with no waiter is ignored instead of resurfacing later", async () => {
    const live = await openServingSocket();
    live.subscriptions.get("p.pair.xfer.down.>")!.push({
      subject: "p.pair.xfer.down.t1.chunk.0", data: new Uint8Array([1]),
    } as never);
    await tick();
    // Nobody is waiting for this chunk (the pull already timed out or was
    // cancelled); the frame must not leave a waiter behind or reach the UI.
    expect(callbacks.onEvent).not.toHaveBeenCalled();
    expect((client as unknown as { downloadWaiters: Map<string, unknown> }).downloadWaiters.size).toBe(0);
  });

  test("a frame delivered after its secure channel was retired is dropped", async () => {
    const live = await openServingSocket();
    const boundary = client as unknown as { secureChannels: WeakMap<object, unknown> };
    jest.spyOn(boundary.secureChannels, "get").mockReturnValue(undefined as never);
    live.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.s1", data: encoder.encode('{"type":"agent_end","data":"{}"}'),
    } as never);
    await tick();
    // Without a channel the payload cannot be opened; the frame is dropped
    // rather than reported as a decode failure or a dead connection.
    expect(callbacks.onEvent).not.toHaveBeenCalled();
    expect(callbacks.onEventDecodeFailure).not.toHaveBeenCalled();
  });

  test("a frame that arrives after the client was closed is not decoded", async () => {
    const live = await openServingSocket();
    jest.mocked(callbacks.onEvent).mockClear();
    // The frame is already queued when the pairing closes, so the lane loop
    // still wakes up for it — with its generation gone.
    live.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.s1", data: encoder.encode('{"type":"agent_end","data":"{}"}'),
    } as never);
    await client.close();
    await tick();
    // A frame still queued on the closed socket belongs to a generation that is
    // gone; projecting it would touch state the user already left.
    expect(callbacks.onEvent).not.toHaveBeenCalled();
  });

  test("a batch yield that outlives its generation stops the loop", async () => {
    const live = await openServingSocket();
    const lane = () => live.subscriptions.get("p.pair.evt.>")!;
    // One frame large enough to trip the byte budget makes the loop yield to
    // input before the next frame is handled.
    lane().push({
      subject: "p.pair.evt.s1",
      data: encoder.encode(`${JSON.stringify({ type: "agent_end", data: "{}" })}${" ".repeat(256 * 1024)}`),
    } as never);
    await tick();
    expect(callbacks.onEvent).toHaveBeenCalledTimes(1);
    lane().push({ subject: "p.pair.evt.s1", data: encoder.encode('{"type":"agent_end","data":"{}"}') } as never);
    await tick();
    await client.close();
    await jest.advanceTimersByTimeAsync(0);
    // The yield is over, but the generation it was batching for is not live any
    // more: the second frame must not be projected.
    expect(callbacks.onEvent).toHaveBeenCalledTimes(1);
  });

  test("a subscription that cannot be attributed to a live generation is refused", async () => {
    const orphan = socket();
    const consume = jest.fn();
    // No attempt has ever run, so neither a candidate nor a serving generation
    // owns the frames this subscription would receive.
    expect(() => (client as unknown as {
      subscribe(c: NatsConnection, g: number, s: string, cat: string, fn: jest.Mock): unknown;
    }).subscribe(orphan.connection, 7, "p.pair.evt.>", "event", consume))
      .toThrow("missing_connection_generation");
    // The refusal happens before the broker is asked for a subscription, so no
    // orphan lane is left behind on the connection.
    expect(orphan.connection.subscribe).not.toHaveBeenCalled();
  });

  test("a frame whose lane moved on between delivery and decoding is dropped", async () => {
    const live = await openServingSocket();
    const consume = jest.fn();
    let checks = 0;
    // The predicate is the one the event lane installs: the record a frame
    // belongs to must still be the current one. Here the lane is replaced
    // between the loop's own check and the delivery.
    (client as unknown as {
      subscribe(c: NatsConnection, g: number, s: string, cat: string, fn: jest.Mock,
                current: () => boolean): unknown;
    }).subscribe(live.connection, 1, "p.pair.evt.s1", "event", consume, () => ++checks <= 1);
    live.subscriptions.get("p.pair.evt.s1")!.push({
      subject: "p.pair.evt.s1", data: encoder.encode('{"type":"agent_end","data":"{}"}'),
    } as never);
    await tick();
    expect(consume).not.toHaveBeenCalled();
  });

  test("a frame whose lane goes stale in the microtask hop is dropped before decoding", async () => {
    const live = await openServingSocket();
    const consume = jest.fn();
    let checks = 0;
    // Same lane contract, one hop later: the record is replaced while the frame
    // waits for its decode turn.
    (client as unknown as {
      subscribe(c: NatsConnection, g: number, s: string, cat: string, fn: jest.Mock,
                current: () => boolean): unknown;
    }).subscribe(live.connection, 1, "p.pair.evt.s1", "event", consume, () => ++checks <= 2);
    live.subscriptions.get("p.pair.evt.s1")!.push({
      subject: "p.pair.evt.s1", data: encoder.encode('{"type":"agent_end","data":"{}"}'),
    } as never);
    await tick();
    expect(consume).not.toHaveBeenCalled();
  });

  test("a presence frame from a restarted desktop is not applied after the pairing closed", async () => {
    const live = await openServingSocket();
    const stale = deferred<unknown>();
    jest.spyOn(client as never, "performHandshake").mockReturnValueOnce(stale.promise as never);
    live.subscriptions.get("p.pair.presence")!.push({
      subject: "p.pair.presence", data: encoder.encode(JSON.stringify({ bridgeInstanceId: "other" })),
    } as never);
    await tick();
    await client.close();
    jest.mocked(callbacks.onFeatures).mockClear();
    jest.mocked(callbacks.onReconnected).mockClear();
    stale.resolve({
      bridgeInstanceId: "other", presence: { bridgeInstanceId: "other", online: true },
      features: ["selective_events_v1"],
    });
    await tick();
    // The re-handshake named a bridge that arrived after the pairing was
    // dropped: adopting it would rebind the phone to a process it may not use.
    expect(callbacks.onReconnected).not.toHaveBeenCalled();
    expect(callbacks.onFeatures).not.toHaveBeenCalledWith(["selective_events_v1"]);
    expect(client.accessIdentity).toBeUndefined();
  });

  test("a presence frame that announces selective events reinstalls the event lane", async () => {
    const live = await openServingSocket();
    client.setVisibleSession("s1");
    jest.spyOn(client as never, "performHandshake").mockResolvedValue({
      bridgeInstanceId: "other", presence: { bridgeInstanceId: "other", online: true },
      features: ["selective_events_v1"],
    } as never);
    live.subscriptions.get("p.pair.presence")!.push({
      subject: "p.pair.presence", data: encoder.encode(JSON.stringify({ bridgeInstanceId: "other" })),
    } as never);
    await tick();
    // The desktop now offers per-session lanes, so the old wildcard lane is
    // replaced by one scoped to the visible session (never both, or every frame
    // would be projected twice).
    expect(live.subscriptions.has("p.pair.evt.s1")).toBe(true);
    expect(callbacks.onFeatures).toHaveBeenLastCalledWith(["selective_events_v1"]);
    expect(callbacks.onReconnected).toHaveBeenCalled();
  });

  test("a second presence frame while a handshake is in flight shares the same handshake", async () => {
    const live = await openServingSocket();
    const inFlight = deferred<unknown>();
    // A fresh mock, so the count covers only the frames pushed below (the
    // initial open's own handshake has already happened).
    const handshake = jest.fn(() => inFlight.promise as never);
    jest.spyOn(client as never, "performHandshake").mockImplementation(handshake as never);
    const presence = () => live.subscriptions.get("p.pair.presence")!.push({
      subject: "p.pair.presence", data: encoder.encode(JSON.stringify({ bridgeInstanceId: "other" })),
    } as never);
    presence();
    await tick();
    presence();
    await tick();
    // A desktop that keeps beating while the first handshake is unanswered must
    // not spend a second identity exchange on the same socket.
    expect(handshake).toHaveBeenCalledTimes(1);
    inFlight.resolve({ bridgeInstanceId: "other", presence: { bridgeInstanceId: "other", online: true } });
    await tick();
    expect(client.accessIdentity).toBe("other");
  });

  test("a ready client whose socket was dropped under it refuses commands loudly", async () => {
    await openServingSocket();
    // The FSM still reports ready while the socket is gone. That inconsistency
    // is a bug elsewhere, but the phone must fail as `not_connected` instead of
    // dereferencing null and reporting a TypeError nobody can act on.
    (client as unknown as { connection: NatsConnection | null }).connection = null;
    await expect(client.request({ type: "list_sessions" })).rejects.toThrow("not_connected");
    await expect(client.uploadChunk("t1", 0, new Uint8Array())).rejects.toThrow("not_connected");
    await expect(client.downloadChunk("t1", 0)).rejects.toThrow("not_connected");
  });
});

