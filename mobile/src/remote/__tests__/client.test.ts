import type { Msg, NatsConnection } from "@nats-io/nats-core";
import { wsconnect } from "@nats-io/nats-core";
import { RemoteClient, type RemoteClientCallbacks } from "../client";
import { RemoteApiError } from "../connectionState";
import { ensureFreshCredentials } from "../pairing";
import type { RemoteCredentials } from "../types";

jest.mock("@nats-io/nats-core", () => ({
  wsconnect: jest.fn(),
  jwtAuthenticator: jest.fn(),
}));
jest.mock("../natsErrors", () => ({ classifyNatsError: () => "transport" }));
jest.mock("../pairing", () => ({
  ensureFreshCredentials: jest.fn(),
  refreshCredentials: jest.fn(),
}));
jest.mock("../codec", () => ({
  jwtExpiry: () => Date.now() / 1000 + 3600,
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
  private waiting = deferred<IteratorResult<T>>();
  private closed = false;
  push(value: T) {
    const waiting = this.waiting;
    this.waiting = deferred<IteratorResult<T>>();
    waiting.resolve({ done: false, value });
  }
  end() {
    this.closed = true;
    this.waiting.resolve({ done: true, value: undefined });
  }
  [Symbol.asyncIterator]() {
    return {
      next: () =>
        this.closed
          ? Promise.resolve({ done: true as const, value: undefined })
          : this.waiting.promise,
    };
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
});
