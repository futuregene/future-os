import { AuthorizationError, ClosedConnectionError, TimeoutError, type Msg, type NatsConnection } from "@nats-io/nats-core";
import { SecureChannel, replyContext } from "@future-os/remote-crypto";
import { RemoteClient, isTransientNatsRequestError, type RemoteClientCallbacks } from "../src/client";
import { ConnectionGeneration } from "../src/connectionGeneration";
import type { RemoteCredentials, RpcResponse } from "../src/types";
import { requestSuite, type RequestCase } from "./requestFixtures";

const encoder = new TextEncoder(), decoder = new TextDecoder();
const settle = async () => { for (let index = 0; index < 80; index++) await Promise.resolve(); };
function category(error: unknown): string {
  if (error instanceof Error && error.message === "client_closed") return "cancelled";
  if (error instanceof AuthorizationError) return "authorization";
  if (isTransientNatsRequestError(error)) return "transport";
  if (error instanceof Error && error.message === "fixture_refused") return "business";
  return "unexpected";
}
function fixture(scenario: RequestCase) {
  const started = Date.now();
  const sent: Array<{ time: number; body: unknown; wire: Uint8Array }> = [];
  const outstanding = new Map<number, { timer: ReturnType<typeof setTimeout>; reject(error: Error): void }>();
  let closed = false;
  const tx = new Uint8Array(32).fill(1), rx = new Uint8Array(32).fill(2), id = new Uint8Array(16).fill(3);
  const server = new SecureChannel(rx, tx, id);
  const channel = new SecureChannel(tx, rx, id);
  const request = async (subject: string, wire: Uint8Array, options: { timeout: number }): Promise<Msg> => {
    const index = sent.length;
    const body: unknown = JSON.parse(decoder.decode(server.open(subject, wire)));
    sent.push({ time: Date.now() - started, body, wire: wire.slice() });
    expect(options.timeout).toBe(requestSuite.requestTimeoutMs);
    const outcome = scenario.outcomes[index];
    if (outcome === undefined) throw new Error("unexpected extra request");
    if (outcome === "transport") throw new TimeoutError();
    if (outcome === "authorization") throw new AuthorizationError("fixture_authorization");
    if (outcome === "timeout") return new Promise((_, reject) => {
      const timer = setTimeout(() => { outstanding.delete(index); reject(new TimeoutError()); }, options.timeout);
      outstanding.set(index, { timer, reject });
    });
    const envelope = outcome === "business" ? { success: false, error: "fixture_refused" } : { success: true, data: requestSuite.response };
    return { data: server.seal(replyContext(subject, wire), encoder.encode(JSON.stringify(envelope))) } as Msg;
  };
  const flush = jest.fn(async () => {});
  const connection = {
    request, flush, isClosed: () => closed,
    close: async () => {
      closed = true;
      for (const pending of outstanding.values()) { clearTimeout(pending.timer); pending.reject(new ClosedConnectionError()); }
      outstanding.clear();
    },
  } as unknown as NatsConnection;
  const callbacks: RemoteClientCallbacks = {
    onCredentials: jest.fn(async () => {}), onConnectionState: jest.fn(), onEvent: jest.fn(),
    onEventDecodeFailure: jest.fn(), onFeatures: jest.fn(), onPresence: jest.fn(),
    onSessions: jest.fn(), onWorkspaces: jest.fn(), onReconnected: jest.fn(), onError: jest.fn(),
  };
  const credentials = { pairId: "pair_fixture", deviceId: "device_fixture", seed: "unused", userJwt: "unused" } as RemoteCredentials;
  const connect = jest.fn(async (): Promise<NatsConnection> => { throw new Error("unexpected connection rebuild"); });
  const refreshCredentials = jest.fn(async () => credentials);
  const client = new RemoteClient(credentials, callbacks, {
    deviceName: () => "fixture", deviceKind: () => "test", randomId: () => "cmd_fixture", refreshCredentials, connect,
  });
  const owner = new ConnectionGeneration(1); owner.activate();
  const internal = client as unknown as { secureChannels: WeakMap<NatsConnection, SecureChannel> };
  Object.assign(client, { connection, activeGeneration: owner, generation: 1, state: "ready", desktopAvailable: true, confirmedBridgeInstanceId: requestSuite.bridgeInstanceId, negotiatedFeatures: new Set(scenario.features) });
  internal.secureChannels.set(connection, channel);
  // Observe the actual recovery entry. Healthy probes run through the role's
  // real recovery path; no replacement implementation is supplied here.
  const recovery = jest.spyOn(client, "recoverNow");
  return { client, started, sent, outstanding, flush, callbacks, connect, refreshCredentials, server, recovery };
}

beforeEach(() => { jest.useFakeTimers(); jest.setSystemTime(0); });
afterEach(() => { jest.useRealTimers(); });

test.each(requestSuite.cases)("shared request $id", async scenario => {
  const f = fixture(scenario);
  let cancel: ReturnType<typeof setTimeout> | undefined;
  try {
    let finished = false;
    let result: RpcResponse<unknown> | undefined, error: unknown;
    const pending = f.client.requestRetry<unknown>(scenario.command, "session_fixture").then(
      value => { result = value; finished = true; },
      failure => { error = failure; finished = true; },
    );
    if (scenario.cancelAtMs !== null) cancel = setTimeout(() => { void f.client.close(); }, scenario.cancelAtMs);
    for (let turn = 0; !finished && turn < 64; turn++) {
      await settle();
      if (finished) break;
      if (jest.getTimerCount() === 0) throw new Error("request is pending without a clock operation");
      await jest.advanceTimersToNextTimerAsync();
    }
    expect(finished).toBe(true);
    await pending;
    const expected = scenario.expected.typescript;
    expect(result ? "success" : category(error)).toBe(expected.outcome);
    if (result) expect(result.data).toEqual(requestSuite.response);
    const completedAt = Date.now() - f.started;
    expect(completedAt).toBeGreaterThanOrEqual(expected.completeAtMs);
    expect(completedAt).toBeLessThanOrEqual(expected.completeAtMs + requestSuite.clockSlackMs.typescript);
    expect(f.sent.map(request => request.time)).toEqual(expected.attemptAtMs);
    expect(f.recovery).toHaveBeenCalledTimes(expected.recoveries);
    expect(f.flush).toHaveBeenCalledTimes(expected.probes);
    const body = { bridgeInstanceId: requestSuite.bridgeInstanceId, ...scenario.command };
    for (const request of f.sent) expect(request.body).toEqual(body);
    // A logical retry keeps its plaintext identity, but must seal a new nonce.
    expect(new Set(f.sent.map(request => Buffer.from(request.wire).toString("base64"))).size).toBe(f.sent.length);
    expect(f.connect).not.toHaveBeenCalled();
    expect(f.refreshCredentials).not.toHaveBeenCalled();
    expect(f.callbacks.onError).not.toHaveBeenCalled();
    expect(f.callbacks.onReconnected).not.toHaveBeenCalled();
    const count = f.sent.length;
    const stopping = f.client.shutdown(1_000);
    await settle();
    expect((await stopping).complete).toBe(true);
    if (cancel !== undefined) clearTimeout(cancel);
    await jest.advanceTimersByTimeAsync(60_000);
    expect(f.sent).toHaveLength(count);
    expect(f.outstanding.size).toBe(0);
    expect(jest.getTimerCount()).toBe(0);
    process.stdout.write(`remote-client shared typescript ${scenario.id} PASS\n`);
  } finally {
    if (cancel !== undefined) clearTimeout(cancel);
    const stopping = f.client.shutdown(0); await settle(); await stopping;
    f.server.destroy();
  }
});
