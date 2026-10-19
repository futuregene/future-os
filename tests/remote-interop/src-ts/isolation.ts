import assert from "node:assert/strict";
import { wsconnect } from "@nats-io/nats-core";
import { RemoteClient, type RemoteClientCallbacks } from "@future-os/remote-client/client";
import type { RemoteCredentials, StreamEvent } from "@future-os/remote-client/types";
import { isolation, suite } from "./contract";
import { bounded, deferred, Harness, type Ready, until } from "./harness";

function controller(ready: Ready) {
  const peer = ready.credentials;
  let stored: RemoteCredentials = {
    pairId: peer.pairId, deviceId: peer.deviceId, seed: peer.nkeySeed, userJwt: peer.userJwt,
    refreshToken: peer.refreshToken, natsWsUrl: peer.natsWsUrl, tokenUrl: peer.tokenUrl,
    expectedDesktopId: peer.desktopId, expectedDesktopPublicKey: ready.desktopNkey,
    secureBundle: JSON.stringify({ identity: { privateKey: peer.secure.privateKey, publicKey: peer.secure.publicKey }, desktopKey: peer.secure.peerPublicKey, secret: peer.secure.secret }),
  };
  const events: StreamEvent[] = [], errors: Error[] = [];
  let sequence = 0;
  function create() {
    const connected = deferred<void>();
    const callbacks: RemoteClientCallbacks = {
      onCredentials: async grant => { stored = grant; },
      onEvent: (event, session) => { if (session === suite.event.sessionId) events.push(event); },
      onEventDecodeFailure: (_session, error) => { errors.push(error); },
      onPresence: () => {}, onSessions: () => {}, onWorkspaces: () => {}, onFeatures: () => {}, onConnectionState: () => {},
      onReconnected: () => connected.resolve(), onError: error => { errors.push(error); connected.reject(error); },
    };
    const client = new RemoteClient(stored, callbacks, {
      deviceName: () => "Isolation TypeScript", deviceKind: () => "test", randomId: prefix => `${prefix}_${++sequence}`,
      refreshCredentials: async grant => grant,
      connect: options => {
        if (typeof options.servers !== "string") throw new Error("fixture requires one explicit endpoint");
        const endpoint = new URL(options.servers);
        if (endpoint.protocol !== "ws:" || endpoint.hostname !== "127.0.0.1") throw new Error("fixture endpoint must be loopback WebSocket");
        return wsconnect(options);
      },
    });
    client.setVisibleSession(suite.event.sessionId);
    return { client, connected: connected.promise };
  }
  return { create, events, errors, stored: () => stored };
}
function report(id: string) { console.log(`remote-interop typescript ${id} PASS`); }

export async function runIsolation(): Promise<void> {
  const hosts: Harness[] = [];
  const roles: RemoteClient[] = [];
  try {
    type Owner = ReturnType<typeof controller>;
    const endpoints: Array<{ input: typeof isolation.endpoints[number]; harness: Harness; owner: Owner; active: ReturnType<Owner["create"]> }> = [];
    for (const input of isolation.endpoints) {
      const { harness, ready } = await Harness.start(input.label);
      hosts.push(harness);
      assert.equal(ready.credentials.desktopId, `interop_${input.label}`);
      const owner = controller(ready), active = owner.create();
      roles.push(active.client);
      await active.client.open(); await bounded(active.connected, 20_000);
      endpoints.push({ input, harness, owner, active });
    }
    const first = endpoints[0]!, second = endpoints[1]!;
    for (const endpoint of endpoints) {
      for (let repeat = 0; repeat < 2; repeat++) {
        assert.deepEqual((await endpoint.active.client.request({ id: isolation.requestId, type: "get_state", message: endpoint.input.message }, "list")).data, { echo: endpoint.input.message, kind: "get_state" });
      }
      const metrics = (await endpoint.harness.command("metrics")).metrics as { executions: Record<string, number>; identityBound: boolean };
      assert.equal(metrics.executions[isolation.requestId], 1);
      assert.equal(metrics.identityBound, true);
    }
    report(isolation.flowIds.requests);
    for (const endpoint of endpoints) {
      const response = await endpoint.active.client.request<{ sessions: Array<{ title: string }> }>({ id: "isolation_catalog", type: "list_sessions" }, "list");
      assert.equal(response.data?.sessions[0]?.title, `interop_${endpoint.input.label}`);
    }
    report(isolation.flowIds.catalog);
    for (const endpoint of endpoints) await endpoint.harness.command("emit", endpoint.input.message);
    await until(async () => endpoints.every(endpoint => endpoint.owner.events.some(event => event.data === endpoint.input.message)));
    for (const endpoint of endpoints) assert.equal(endpoint.owner.events.every(event => event.data === endpoint.input.message), true);
    report(isolation.flowIds.events);
    assert.equal((await first.active.client.shutdown(5_000)).complete, true);
    assert.deepEqual((await second.active.client.request({ id: "isolation_survivor", type: "get_state", message: second.input.message }, "list")).data, { echo: second.input.message, kind: "get_state" });
    const firstMetrics = (await first.harness.command("metrics")).metrics as { executions: Record<string, number> };
    assert.equal(firstMetrics.executions.isolation_survivor, undefined);
    report(isolation.flowIds.disconnect);
    first.active = first.owner.create(); roles.push(first.active.client);
    await first.active.client.open(); await bounded(first.active.connected, 20_000);
    assert.deepEqual((await first.active.client.request({ id: "isolation_restored", type: "get_state", message: first.input.message }, "list")).data, { echo: first.input.message, kind: "get_state" });
    const secondMetrics = (await second.harness.command("metrics")).metrics as { executions: Record<string, number> };
    assert.equal(secondMetrics.executions.isolation_restored, undefined);
    // The survivor must still answer after the other role has been replaced.
    assert.deepEqual((await second.active.client.request({ id: "isolation_still_live", type: "get_state", message: second.input.message }, "list")).data, { echo: second.input.message, kind: "get_state" });
    for (const endpoint of endpoints) {
      let bundle: unknown;
      try { bundle = JSON.parse(endpoint.owner.stored().secureBundle!); } catch { throw new Error("invalid stored isolation bundle"); }
      assert.equal(typeof bundle === "object" && bundle !== null && !Object.hasOwn(bundle, "secret"), true);
      assert.deepEqual(endpoint.owner.errors.map(error => error.message), []);
    }
    report(isolation.flowIds.reconnect);
  } finally {
    const results = await Promise.allSettled(roles.map(async client => {
      if (!(await client.shutdown(5_000)).complete) throw new Error("isolation role cleanup incomplete");
    }));
    const hostResults = await Promise.allSettled(hosts.map(host => host.close()));
    if ([...results, ...hostResults].some(result => result.status === "rejected")) throw new Error("isolation cleanup failed");
  }
}
