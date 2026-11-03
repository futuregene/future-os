import type { Msg, NatsConnection, Status } from "@nats-io/nats-core";
import { QueuedIteratorImpl } from "@nats-io/nats-core/internal";
import { SecureChannel, replyContext } from "@future-os/remote-crypto";
import { RemoteClient, type RemoteClientCallbacks } from "../src/client";
import { RemoteApiError } from "../src/connectionState";
import { TaskScope } from "../src/taskScope";
import { downloadPrepared } from "../src/transfer";
import type { ClientHost } from "../src/host";
import type { RemoteCredentials } from "../src/types";

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const settle = async () => { for (let index = 0; index < 80; index++) await Promise.resolve(); };
const encoder = new TextEncoder();
const token = (seconds = 3600) => `e30.${Buffer.from(JSON.stringify({ exp: Math.floor(Date.now() / 1000) + seconds })).toString("base64url")}.sig`;

function fixture(expiry = 3600) {
  const credentials: RemoteCredentials = {
    pairId: "pair", deviceId: "device", userJwt: token(expiry), seed: "unused",
    natsWsUrl: "wss://example.test", tokenUrl: "https://example.test/token",
    refreshToken: "refresh", expectedDesktopId: "desktop", expectedDesktopPublicKey: "unused",
  };
  const callbacks: RemoteClientCallbacks = {
    onCredentials: jest.fn(async () => {}), onEvent: jest.fn(), onEventDecodeFailure: jest.fn(),
    onPresence: jest.fn(), onSessions: jest.fn(), onWorkspaces: jest.fn(), onFeatures: jest.fn(),
    onConnectionState: jest.fn(), onReconnected: jest.fn(), onError: jest.fn(),
  };
  const subscriptions = new Set<QueuedIteratorImpl<Msg>>();
  const status = new QueuedIteratorImpl<Status>();
  const tx = new Uint8Array(32).fill(1), rx = new Uint8Array(32).fill(2), id = new Uint8Array(16).fill(3);
  const server = new SecureChannel(rx, tx, id);
  const stopQueues = () => { for (const queue of subscriptions) queue.stop(); status.stop(); };
  const request = jest.fn(async (subject: string, wire: Uint8Array) => {
    server.open(subject, wire);
    const data = encoder.encode(JSON.stringify({ success: true, data: { ok: true } }));
    return { data: server.seal(replyContext(subject, wire), data) } as Msg;
  });
  const close = jest.fn(async () => { stopQueues(); });
  const connection = {
    subscribe: () => {
      const queue = new QueuedIteratorImpl<Msg>();
      Object.assign(queue, { unsubscribe: () => queue.stop() });
      subscriptions.add(queue);
      return queue;
    },
    status: () => status, request, close, flush: jest.fn(async () => {}), isClosed: () => false,
  } as unknown as NatsConnection;
  const connect = jest.fn(async () => connection);
  const refreshCredentials = jest.fn(async () => ({ ...credentials, userJwt: token() }));
  const host: ClientHost = {
    connect, refreshCredentials, deviceName: () => "Test host", deviceKind: () => "test", randomId: prefix => `${prefix}_id`,
  };
  const client = new RemoteClient(credentials, callbacks, host);
  const internal = client as unknown as {
    runHandshake(connection: NatsConnection): Promise<unknown>;
    activateSecureChannel(connection: NatsConnection): Promise<void>;
    secureChannels: WeakMap<NatsConnection, SecureChannel>;
    runDownloadChunk(connection: NatsConnection, transferId: string, index: number): Promise<Uint8Array>;
    refreshToken(): Promise<void>;
  };
  // Lifecycle tests keep real role loops/requests and explicit memory IO. The
  // Noise pairing exchange is covered separately by secureClient and interop.
  jest.spyOn(internal, "runHandshake").mockResolvedValue({ bridgeInstanceId: "bridge", presence: { online: true, bridgeInstanceId: "bridge" }, features: [] });
  jest.spyOn(internal, "activateSecureChannel").mockResolvedValue(undefined);
  internal.secureChannels.set(connection, new SecureChannel(tx, rx, id));
  return { client, internal, credentials, callbacks, connection, connect, request, close, refreshCredentials, stopQueues };
}

beforeEach(() => { jest.useFakeTimers(); });
afterEach(() => { jest.restoreAllMocks(); jest.useRealTimers(); });

test("task registration precedes a synchronous host callback that starts waiting", async () => {
  const scope = new TaskScope();
  const work = deferred<void>();
  let observation!: ReturnType<TaskScope["wait"]>;
  const task = scope.run("host", () => { observation = scope.wait(0); return work.promise; });
  expect(await observation).toEqual({ complete: false, pendingTasks: ["host"] });
  work.resolve();
  await task;
  expect(await scope.wait(0)).toEqual({ complete: true, pendingTasks: [] });
});

test("a timeout keeps real tasks and their later children owned for another wait", async () => {
  const scope = new TaskScope();
  const first = deferred<void>(), child = deferred<void>();
  const parent = scope.run("parent", async () => { await first.promise; await scope.run("child", () => child.promise); });
  const waiting = scope.wait(100);
  await jest.advanceTimersByTimeAsync(100);
  expect(await waiting).toEqual({ complete: false, pendingTasks: ["parent"] });
  first.resolve();
  await settle();
  expect((await scope.wait(0)).pendingTasks).toEqual(["parent", "child"]);
  child.resolve();
  await parent;
  expect(await scope.wait(0)).toEqual({ complete: true, pendingTasks: [] });
});

test("task rejection is observed and represents real completion", async () => {
  const scope = new TaskScope();
  const job = scope.run("host", () => { throw new Error("host_failed"); });
  await expect(job).rejects.toThrow("host_failed");
  expect(await scope.wait(0)).toEqual({ complete: true, pendingTasks: [] });
});

test("shutdown waits for a late host connection and its actual close after an observation timeout", async () => {
  const f = fixture();
  const connecting = deferred<NatsConnection>(), closing = deferred<void>();
  f.connect.mockReturnValue(connecting.promise);
  f.close.mockReturnValue(closing.promise);
  const opening = f.client.open();
  await settle();
  expect(f.connect).toHaveBeenCalledTimes(1);
  const stopping = f.client.shutdown(100);
  await jest.advanceTimersByTimeAsync(100);
  expect((await stopping).complete).toBe(false);
  connecting.resolve(f.connection);
  await settle();
  expect(f.close).toHaveBeenCalledTimes(1);
  expect((await f.client.waitTasks(0)).pendingTasks).toContain("connection-close");
  expect(f.callbacks.onReconnected).not.toHaveBeenCalled();
  closing.resolve();
  await opening;
  expect(await f.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
  await f.client.open();
  await f.client.recoverNow("network-restored");
  expect(f.connect).toHaveBeenCalledTimes(1);
});

test("close remains immediate while shutdown observes SDK close and subscription/status completion", async () => {
  const f = fixture();
  await f.client.open();
  const closing = deferred<void>();
  f.close.mockImplementation(async () => { await closing.promise; f.stopQueues(); });
  await f.client.close();
  await f.client.close();
  expect(f.close).toHaveBeenCalledTimes(1);
  const stopping = f.client.shutdown(50);
  await jest.advanceTimersByTimeAsync(50);
  const result = await stopping;
  expect(result.complete).toBe(false);
  expect(result.pendingTasks).toContain("connection-close");
  closing.resolve();
  expect(await f.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
});

test("connect outliving the normal opening deadline is still visible to final shutdown", async () => {
  const f = fixture();
  const connecting = deferred<NatsConnection>();
  f.connect.mockReturnValue(connecting.promise);
  const opening = f.client.open();
  await settle();
  await jest.advanceTimersByTimeAsync(20_000);
  await opening;
  expect((await f.client.shutdown(0)).pendingTasks).toContain("probe-or-setup");
  connecting.resolve(f.connection);
  expect(await f.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
  expect(f.close).toHaveBeenCalledTimes(1);
  expect(f.callbacks.onReconnected).not.toHaveBeenCalled();
});

test("shutdown does not declare an in-flight SDK command complete when caller cancellation begins", async () => {
  const f = fixture();
  await f.client.open();
  const reply = deferred<Msg>();
  f.request.mockReturnValueOnce(reply.promise);
  const command = f.client.request({ type: "get_state", sessionId: "s" });
  const commandResult = expect(command).rejects.toThrow("sdk_closed");
  await settle();
  const state = await f.client.shutdown(0);
  expect(state.complete).toBe(false);
  expect(state.pendingTasks).toContain("secure-request");
  reply.reject(new Error("sdk_closed"));
  await commandResult;
  expect(await f.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
});

test("native hash remains owned after cancellation until the actual native work returns", async () => {
  const f = fixture();
  await f.client.open();
  jest.spyOn(f.internal, "runDownloadChunk").mockResolvedValue(new Uint8Array([1, 2]));
  const native = deferred<string>();
  const file = { bytes: [] as number[] };
  const remove = jest.fn();
  const hash = jest.fn(() => native.promise);
  const downloading = downloadPrepared(f.client, {
    transferId: "file", size: 2, chunkBytes: 2, contentHash: "hash", name: "file", mimeType: "text/plain", previewKind: "text", variant: "preview",
  }, {
    maxFileBytes: 10, verifiedCached: async () => null,
    createDownload: () => ({ file, handle: { writeBytes: bytes => file.bytes.push(...bytes), close: () => {} } }),
    size: value => value.bytes.length, remove, hash,
  });
  const outcome = expect(downloading).rejects.toThrow("transfer_cancelled");
  await settle();
  expect(hash).toHaveBeenCalledTimes(1);
  const state = await f.client.shutdown(0);
  expect(state.complete).toBe(false);
  expect(state.pendingTasks).toContain("file-transfer");
  native.resolve("hash");
  await outcome;
  expect(remove).toHaveBeenCalledTimes(1);
  expect(await f.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
});

test("pending credential persistence is awaited and cannot connect after shutdown", async () => {
  const f = fixture(30);
  const saving = deferred<void>();
  jest.mocked(f.callbacks.onCredentials).mockReturnValue(saving.promise);
  const opening = f.client.open();
  await settle();
  expect(f.callbacks.onCredentials).toHaveBeenCalledTimes(1);
  expect((await f.client.shutdown(0)).complete).toBe(false);
  saving.resolve();
  await opening;
  expect(await f.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
  expect(f.connect).not.toHaveBeenCalled();
});

test("an in-flight refresh cannot publish credentials or reopen after shutdown", async () => {
  const f = fixture();
  await f.client.open();
  const refreshing = deferred<RemoteCredentials>();
  f.refreshCredentials.mockReturnValue(refreshing.promise);
  const work = f.internal.refreshToken();
  await settle();
  expect((await f.client.shutdown(0)).pendingTasks).toContain("credential-refresh");
  refreshing.resolve({ ...f.credentials, userJwt: token() });
  await work;
  expect((await f.client.waitTasks(100)).complete).toBe(true);
  expect(f.callbacks.onCredentials).not.toHaveBeenCalled();
  expect(f.connect).toHaveBeenCalledTimes(1);
});

test("one instance's unfinished native work does not delay another instance's shutdown", async () => {
  const a = fixture(), b = fixture();
  const native = deferred<void>();
  const work = a.client.runTransfer(async signal => { await native.promise; return signal.aborted; });
  expect(await b.client.shutdown(100)).toEqual({ complete: true, pendingTasks: [] });
  expect((await a.client.shutdown(0)).complete).toBe(false);
  native.resolve();
  expect(await work).toBe(true);
  expect(await a.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
});


test("closed runtime refuses new file work without invoking the host callback", async () => {
  const f = fixture();
  expect((await f.client.shutdown(100)).complete).toBe(true);
  const operation = jest.fn(async () => "unexpected");
  await expect(f.client.runTransfer(operation)).rejects.toThrow("client_closed");
  expect(operation).not.toHaveBeenCalled();
  expect((await f.client.waitTasks(100)).complete).toBe(true);
});

test("a synchronous connecting-state callback can stop before host IO begins", async () => {
  const f = fixture();
  jest.mocked(f.callbacks.onConnectionState).mockImplementation(state => {
    if (state === "connecting") void f.client.close();
  });
  await f.client.open();
  expect(await f.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
  expect(f.connect).not.toHaveBeenCalled();
  expect(f.refreshCredentials).not.toHaveBeenCalled();
  expect(jest.getTimerCount()).toBe(0);
});

test.each(["catalog", "presence", "features", "ready"])("stopping from the %s callback cannot emit readiness or arm a new timer", async stage => {
  const f = fixture();
  if (stage === "catalog") f.callbacks.onCatalogEpoch = () => { void f.client.close(); };
  if (stage === "presence") jest.mocked(f.callbacks.onPresence).mockImplementation(() => { void f.client.close(); });
  if (stage === "features") jest.mocked(f.callbacks.onFeatures).mockImplementation(() => { void f.client.close(); });
  if (stage === "ready") jest.mocked(f.callbacks.onConnectionState).mockImplementation(state => {
    if (state === "ready") void f.client.close();
  });
  await f.client.open();
  expect(await f.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
  expect(f.callbacks.onReconnected).not.toHaveBeenCalled();
  expect(f.close).toHaveBeenCalledTimes(1);
  expect(jest.getTimerCount()).toBe(0);
});

test("a synchronous refreshing-state stop cannot start a new credential service call", async () => {
  const f = fixture();
  await f.client.open();
  jest.mocked(f.callbacks.onConnectionState).mockImplementation(state => {
    if (state === "refreshing") void f.client.close();
  });
  await f.internal.refreshToken();
  expect(await f.client.waitTasks(100)).toEqual({ complete: true, pendingTasks: [] });
  expect(f.refreshCredentials).not.toHaveBeenCalled();
  expect(f.connect).toHaveBeenCalledTimes(1);
});

test("a fatal open leaves connectionState failed, so a runtime rebuilds instead of probing", async () => {
  const f = fixture();
  try {
    expect(f.client.connectionState).toBe("unpaired");
    f.connect.mockRejectedValueOnce(
      new RemoteApiError("misconfigured", "remote_service_misconfigured", 500),
    );
    await f.client.open().catch(() => undefined);
    // The state machine keeps this client terminal: every request now answers
    // `communication_frozen`, and the runtime has to see that rather than keep
    // probing. A retry inside the same object recovers nothing.
    expect(f.client.connectionState).toBe("failed");
    await f.client.recoverNow("request-failure");
    expect(f.connect).toHaveBeenCalledTimes(1);
  } finally {
    await f.client.close();
  }
});
