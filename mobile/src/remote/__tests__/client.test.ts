import type { Msg, NatsConnection } from "@nats-io/nats-core";
import { wsconnect } from "@nats-io/nats-core";
import { RemoteClient, type RemoteClientCallbacks } from "../client";
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
  });

  afterEach(async () => {
    await client.close();
    jest.useRealTimers();
    jest.restoreAllMocks();
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
    const recovering = client.recoverNow("network-changed");
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
    await client.recoverNow("network-changed");
    expect(replacement.close).toHaveBeenCalled();
    expect(old.close).not.toHaveBeenCalled();
    old.subscriptions.get("p.pair.evt.>")!.push({
      subject: "p.pair.evt.session",
      data: new TextEncoder().encode('{"type":"agent_end"}'),
    } as Msg);
    await tick();
    expect(callbacks.onEvent).toHaveBeenCalledTimes(1);
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
    const recovering = client.recoverNow("network-changed");
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
    const recovering = client.recoverNow("network-changed");
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
