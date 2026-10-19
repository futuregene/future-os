import input from "../fixtures/recovery.json";
import { RemoteClient, type RemoteClientCallbacks } from "../src/client";
import type { RemoteCredentials } from "../src/types";
import type { NatsConnection } from "@nats-io/nats-core";

function record(value: unknown, keys: string[]): asserts value is Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(key => !keys.includes(key)) || keys.some(key => !Object.hasOwn(value, key))) throw new Error("invalid recovery fixture fields");
}
record(input, ["version", "suite", "nowMs", "rustObservationMs", "typescriptBudgetMs", "cases"]);
if (input.version !== 1 || input.suite !== "client.recovery") throw new Error("unsupported recovery fixture");
if (!Number.isSafeInteger(input.nowMs) || input.nowMs <= 0 || !Number.isSafeInteger(input.rustObservationMs) || input.rustObservationMs <= 0 || input.rustObservationMs > 10_000 || !Number.isSafeInteger(input.typescriptBudgetMs) || input.typescriptBudgetMs <= 0 || input.typescriptBudgetMs > 600_000 || !Array.isArray(input.cases) || !input.cases.length) throw new Error("invalid recovery clock/cases");
const ids = new Set<string>();
for (const item of input.cases) {
  record(item, ["id", "closeAfterAttempts", "expected", "difference"]);
  record(item.expected, ["rust", "typescript"]);
  if (typeof item.id !== "string" || !item.id.startsWith("client.recovery.") || ids.has(item.id) || typeof item.difference !== "string" || !item.difference.trim()) throw new Error("invalid recovery identity/difference");
  ids.add(item.id);
  for (const count of [item.expected.rust, item.expected.typescript, ...(item.closeAfterAttempts === null ? [] : [item.closeAfterAttempts])]) if (!Number.isSafeInteger(count) || Number(count) < 0 || Number(count) > 100) throw new Error("invalid recovery count");
  if (item.closeAfterAttempts !== null && (item.expected.rust !== item.closeAfterAttempts || item.expected.typescript !== item.closeAfterAttempts)) throw new Error("inconsistent recovery stop");
}
const settle = async () => { for (let i = 0; i < 80; i++) await Promise.resolve(); };
beforeEach(() => {
  jest.useFakeTimers(); jest.setSystemTime(input.nowMs);
  jest.spyOn(Math, "random").mockReturnValue(0.5);
  jest.spyOn(console, "warn").mockImplementation(() => {});
});
afterEach(() => { jest.restoreAllMocks(); jest.useRealTimers(); });

test.each(input.cases)("shared recovery $id", async scenario => {
  const grant: RemoteCredentials = {
    pairId: "pair_fixture", deviceId: "device_fixture", seed: "unused", refreshToken: "unused",
    userJwt: `e30.${Buffer.from(JSON.stringify({ exp: input.nowMs / 1000 + 3600 })).toString("base64url")}.sig`,
    natsWsUrl: "wss://fixture.invalid", tokenUrl: "https://fixture.invalid/token", expectedDesktopId: "desktop_fixture", expectedDesktopPublicKey: "unused",
  };
  const callbacks: RemoteClientCallbacks = {
    onCredentials: jest.fn(), onEvent: jest.fn(), onEventDecodeFailure: jest.fn(), onFeatures: jest.fn(),
    onPresence: jest.fn(), onSessions: jest.fn(), onWorkspaces: jest.fn(), onConnectionState: jest.fn(), onReconnected: jest.fn(), onError: jest.fn(),
  };
  const attempts: number[] = [];
  const connect = jest.fn(async (): Promise<NatsConnection> => { attempts.push(Date.now() - input.nowMs); throw new Error("fixture_transport_unavailable"); });
  const client = new RemoteClient(grant, callbacks, {
    deviceName: () => "fixture", deviceKind: () => "test", randomId: () => "fixture_id", refreshCredentials: async value => value, connect,
  });
  // Establish the precondition of a previously ready role. Connection recovery,
  // retries, deadline and stop run through actual open()/shutdown() code. This
  // does not simulate a successful handshake or replace recovery scheduling.
  Object.assign(client, { everReady: true, state: "ready", desktopAvailable: true });
  try {
    if (scenario.closeAfterAttempts === 0) await client.shutdown(0);
    await client.open(); await settle();
    if (scenario.closeAfterAttempts !== null) {
      for (let guard = 0; attempts.length < scenario.closeAfterAttempts; guard++) {
        expect(guard).toBeLessThan(100);
        await jest.advanceTimersToNextTimerAsync(); await settle();
      }
      const cleanup = client.shutdown(1_000); await settle();
      expect((await cleanup).complete).toBe(true);
    } else {
      expect((client as unknown as { recoveryDeadline: number }).recoveryDeadline).toBe(input.nowMs + input.typescriptBudgetMs);
      await jest.advanceTimersByTimeAsync(input.typescriptBudgetMs); await settle();
      expect((client as unknown as { state: string }).state).toBe("failed");
      expect(attempts.every(at => at < input.typescriptBudgetMs)).toBe(true);
      expect(callbacks.onError).toHaveBeenLastCalledWith(expect.objectContaining({ message: "recovery_timeout" }));
    }
    expect(connect).toHaveBeenCalledTimes(scenario.expected.typescript);
    expect(attempts.length).toBe(scenario.expected.typescript);
    expect((await client.waitTasks(0)).complete).toBe(true);
    await jest.advanceTimersByTimeAsync(600_000); await client.open(); await settle();
    expect(connect).toHaveBeenCalledTimes(scenario.expected.typescript);
    expect(jest.getTimerCount()).toBe(0);
    process.stdout.write(`remote-client shared typescript ${scenario.id} PASS\n`);
  } finally {
    const cleanup = client.shutdown(1_000); await settle();
    expect((await cleanup).complete).toBe(true);
  }
});
