import { RemoteClient, type RemoteClientCallbacks } from "../src/client";
import { ensureFreshCredentials } from "../src/pairing";
import type { NatsConnection } from "@nats-io/nats-core";
import type { RemoteCredentials } from "../src/types";
import { credentialSuite } from "./credentialFixtures";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
const settle = async () => { for (let index = 0; index < 80; index++) await Promise.resolve(); };
const token = (expiry: number) => `header.${Buffer.from(JSON.stringify({ exp: expiry })).toString("base64url")}.signature`;
const credentials = (expiry: number): RemoteCredentials => ({
  pairId: "pair_fixture", deviceId: "device_fixture", seed: "unused", userJwt: token(expiry), refreshToken: "unused",
  natsWsUrl: "wss://fixture.invalid", tokenUrl: "https://fixture.invalid/token", expectedDesktopId: "desktop_fixture", expectedDesktopPublicKey: "unused",
});
function actor() {
  const current = credentials(credentialSuite.nowSec + 3600);
  const fresh = { ...current, userJwt: token(credentialSuite.nowSec + 7200) };
  let stored = current;
  const response = deferred<RemoteCredentials>(), entered = deferred<void>();
  const onCredentials = jest.fn(async (grant: RemoteCredentials) => { stored = grant; });
  const callbacks: RemoteClientCallbacks = {
    onCredentials, onConnectionState: jest.fn(), onEvent: jest.fn(), onEventDecodeFailure: jest.fn(),
    onFeatures: jest.fn(), onPresence: jest.fn(), onSessions: jest.fn(), onWorkspaces: jest.fn(), onError: jest.fn(), onReconnected: jest.fn(),
  };
  let signal: AbortSignal | undefined;
  const refresh = jest.fn((_grant: RemoteCredentials, owner?: AbortSignal) => {
    signal = owner; entered.resolve();
    // Deliberately uncooperative host promise: abort must fence adoption, and
    // role ownership must remain until this actual service result is supplied.
    return response.promise;
  });
  const connect = jest.fn(async (): Promise<NatsConnection> => { throw new Error("unexpected credential-test network connection"); });
  const client = new RemoteClient(current, callbacks, {
    deviceName: () => "fixture", deviceKind: () => "test", randomId: () => "fixture_id", refreshCredentials: refresh, connect,
  });
  Object.assign(client, { state: "ready" });
  const internal = client as unknown as { refreshToken(): Promise<void>; cancelAttempt(): void; stopped: boolean; credentials: RemoteCredentials };
  // Renewal's continuation is observed, not replaced with a second renewal
  // policy. Actual handshakes/connections live in the standalone interop suite.
  const continuation = jest.spyOn(client, "open").mockResolvedValue(undefined);
  let finished = false;
  const work = internal.refreshToken();
  void work.then(() => { finished = true; }, () => { finished = true; });
  return { client, internal, callbacks, entered, response, fresh, refresh, onCredentials, continuation, connect, work, finished: () => finished, stored: () => stored, signal: () => signal };
}

beforeEach(() => { jest.useFakeTimers(); jest.setSystemTime(credentialSuite.nowSec * 1000); });
afterEach(() => { jest.restoreAllMocks(); jest.useRealTimers(); });

test.each(credentialSuite.cases)("shared credentials $id", async scenario => {
  if (scenario.kind === "expiry") {
    const current = credentials(scenario.expiry);
    if (scenario.malformed) current.userJwt = "malformed_fixture";
    const fresh = { ...current, userJwt: token(credentialSuite.nowSec + 7200) };
    const refresh = jest.fn(async () => fresh);
    let decision: string;
    try {
      const result = await ensureFreshCredentials(current, undefined, refresh);
      decision = result === current ? "no_refresh" : "refresh";
      // Report booleans on failure instead of dumping even fixture grants.
      expect(result === current || result === fresh).toBe(true);
    } catch (error) {
      if (!(error instanceof Error) || error.message !== "invalid_jwt") throw error;
      decision = "invalid";
    }
    expect(decision).toBe(scenario.expected.typescript);
    expect(refresh).toHaveBeenCalledTimes(decision === "refresh" ? 1 : 0);
  } else {
    const actors = Array.from({ length: scenario.instances }, actor);
    const released = new Set<number>();
    try {
      await settle();
      for (const instance of actors) expect(instance.refresh).toHaveBeenCalledTimes(1);
      await Promise.all(actors.map(actor => actor.entered.promise));
      for (const action of scenario.actions) {
        const instance = actors[action.target]!;
        switch (action.type) {
          case "close": await instance.client.shutdown(0); break;
          case "retire": instance.client.setAppActive(false); break;
          case "supersede": instance.internal.cancelAttempt(); break;
          case "release": released.add(action.target); instance.response.resolve(instance.fresh); break;
        }
        await settle();
        if (action.type !== "release" && !released.has(action.target)) {
          expect(instance.signal()?.aborted).toBe(true);
          expect((await instance.client.waitTasks(0)).complete).toBe(false);
        }
      }
      for (const instance of actors) expect(instance.finished()).toBe(true);
      await Promise.all(actors.map(actor => actor.work));
      for (const [index, instance] of actors.entries()) {
        expect(instance.refresh).toHaveBeenCalledTimes(1);
        expect(instance.onCredentials).toHaveBeenCalledTimes(scenario.expected.writes[index]!);
        expect(instance.stored().userJwt === instance.fresh.userJwt).toBe(scenario.expected.fresh[index]);
        expect(instance.internal.credentials.userJwt === instance.fresh.userJwt).toBe(scenario.expected.fresh[index]);
        expect(instance.continuation).toHaveBeenCalledTimes(scenario.expected.continuations[index]!);
        expect(instance.internal.stopped).toBe(scenario.expected.closed[index]);
        expect(instance.connect).not.toHaveBeenCalled();
        expect(instance.callbacks.onError).not.toHaveBeenCalled();
        expect((await instance.client.waitTasks(0)).complete).toBe(true);
        const stopping = instance.client.shutdown(1_000); await settle();
        expect((await stopping).complete).toBe(true);
      }
      await jest.advanceTimersByTimeAsync(60_000);
      for (const [index, instance] of actors.entries()) expect(instance.onCredentials).toHaveBeenCalledTimes(scenario.expected.writes[index]!);
      expect(jest.getTimerCount()).toBe(0);
    } finally {
      for (const instance of actors) {
        instance.response.resolve(instance.fresh);
        const stopping = instance.client.shutdown(0); await settle(); await stopping;
      }
    }
  }
  process.stdout.write(`remote-client shared typescript ${scenario.id} PASS\n`);
});
