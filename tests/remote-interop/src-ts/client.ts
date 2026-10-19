import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { wsconnect } from "@nats-io/nats-core";
import { RemoteClient, type RemoteClientCallbacks } from "@future-os/remote-client/client";
import { downloadPrepared, prepareDownload, uploadAttachments } from "@future-os/remote-client/transfer";
import type { RemoteCredentials, RpcResponse, StreamEvent } from "@future-os/remote-client/types";
import { suite } from "./contract";
import { bounded, deferred, Harness, until } from "./harness";
import { runIsolation } from "./isolation";
import { runTransferStop } from "./transferStop";

function report(id: string): void { console.log(`remote-interop typescript ${id} PASS`); }
async function main(): Promise<void> {
  const { harness, ready } = await Harness.start();
  let client: RemoteClient | undefined;
  const peer = ready.credentials;
  let credentials: RemoteCredentials = {
    pairId: peer.pairId, deviceId: peer.deviceId, seed: peer.nkeySeed, userJwt: peer.userJwt,
    refreshToken: peer.refreshToken, natsWsUrl: peer.natsWsUrl, tokenUrl: peer.tokenUrl,
    expectedDesktopId: peer.desktopId, expectedDesktopPublicKey: ready.desktopNkey,
    secureBundle: JSON.stringify({ identity: { privateKey: peer.secure.privateKey, publicKey: peer.secure.publicKey }, desktopKey: peer.secure.peerPublicKey, secret: peer.secure.secret }),
  };
  let sequence = 0;
  const events: StreamEvent[] = [];
  const errors: Error[] = [];
  function instance() {
    const connected = deferred<void>();
    const callbacks: RemoteClientCallbacks = {
      onCredentials: async fresh => { credentials = fresh; },
      onEvent: (event, session) => { if (session === suite.event.sessionId) events.push(event); },
      onEventDecodeFailure: (_session, error) => { errors.push(error); },
      onPresence: () => {}, onSessions: () => {}, onWorkspaces: () => {}, onFeatures: () => {},
      onConnectionState: () => {}, onReconnected: () => connected.resolve(),
      onError: error => { errors.push(error); connected.reject(error); },
    };
    const role = new RemoteClient(credentials, callbacks, {
      deviceName: () => "Standalone TypeScript", deviceKind: () => "test", randomId: prefix => `${prefix}_${++sequence}`,
      refreshCredentials: async fresh => fresh,
      connect: options => {
        if (typeof options.servers !== "string") throw new Error("fixture requires one explicit endpoint");
        const endpoint = new URL(options.servers);
        if (endpoint.protocol !== "ws:" || endpoint.hostname !== "127.0.0.1") throw new Error("plaintext fixture endpoint must be IPv4 loopback");
        return wsconnect(options);
      },
    });
    role.setVisibleSession(suite.event.sessionId);
    return { role, connected: connected.promise };
  }
  async function metrics(): Promise<{ executions: Record<string, number>; downloads: number; identityBound: boolean }> {
    const result = await harness.command("metrics");
    return result.metrics as { executions: Record<string, number>; downloads: number; identityBound: boolean };
  }
  try {
    let current = instance();
    client = current.role;
    await client.open();
    await bounded(current.connected, 20_000);
    assert.equal((await metrics()).identityBound, true);
    let storedBundle: unknown;
    try { storedBundle = JSON.parse(credentials.secureBundle!); }
    catch { throw new Error("malformed stored secure bundle"); }
    // A failing assertion must print only a boolean, never the invitation PSK.
    assert.equal(typeof storedBundle === "object" && storedBundle !== null && !Object.hasOwn(storedBundle, "secret"), true);
    report(suite.flowIds.firstPair);
    for (const scenario of suite.cases) {
      for (const step of scenario.steps) {
        try {
          const response: RpcResponse<unknown> = await client.request<unknown>(step.command, "list");
          assert.equal(step.error, undefined, `expected refusal: ${scenario.id}`);
          assert.deepEqual(response.data, step.data, scenario.id);
        } catch (error) {
          if (step.error === undefined) throw error;
          assert.equal(error instanceof Error ? error.message : String(error), step.error, scenario.id);
        }
      }
      const commandId = scenario.steps[0]!.command.id;
      assert.equal((await metrics()).executions[commandId], scenario.expectedExecutions, scenario.id);
      report(scenario.id);
    }
    await harness.command("emit");
    await until(async () => events.some(event => event.type === suite.event.type));
    const event = events.find(event => event.type === suite.event.type)!;
    assert.equal(event.runId, suite.event.runId);
    assert.equal(event.idx, suite.event.idx);
    assert.equal(event.data, suite.event.data);
    report(suite.flowIds.event);

    const bytes = Uint8Array.from(suite.fileBytes);
    let readerClosed = false;
    const uploaded = await uploadAttachments(client, [{
      name: "fixture.bin", mimeType: "application/octet-stream", kind: "file", originalSize: bytes.length, transferSize: bytes.length,
    }], {
      openUpload: () => {
        let offset = 0;
        return { get offset() { return offset; }, readBytes: count => { const value = bytes.slice(offset, offset + count); offset += value.length; return value; }, close: () => { readerClosed = true; } };
      },
    });
    assert.equal(readerClosed, true);
    assert.equal(uploaded[0]!.contentHash, createHash("sha256").update(bytes).digest("hex"));
    const info = await prepareDownload(client, suite.event.sessionId, { path: "fixture.bin", name: "fixture.bin" }, { verifiedCached: async () => null }, "original");
    const file = { bytes: [] as number[], closed: false };
    const downloaded = await downloadPrepared(client, info, {
      maxFileBytes: 10 * 1024 * 1024, verifiedCached: async () => null,
      createDownload: () => ({ file, handle: { writeBytes: chunk => { file.bytes.push(...chunk); }, close: () => { file.closed = true; } } }),
      size: output => output.bytes.length,
      remove: output => { output.bytes = []; },
      hash: async output => createHash("sha256").update(Uint8Array.from(output.bytes)).digest("hex"),
    });
    assert.deepEqual(downloaded.bytes, [...bytes]);
    assert.equal(file.closed, true);
    await until(async () => (await metrics()).downloads === 0);
    report(suite.flowIds.transfer);

    assert.equal((await client.shutdown(5_000)).complete, true);
    current = instance();
    client = current.role;
    await client.open();
    await bounded(current.connected, 20_000);
    assert.deepEqual((await client.request({ id: "reconnect", type: "get_state", message: "IK" }, "list")).data, { echo: "IK", kind: "get_state" });
    assert.equal((await metrics()).identityBound, true);
    report(suite.flowIds.reconnect);
    assert.equal((await client.shutdown(5_000)).complete, true);
    assert.deepEqual(errors.map(error => error.message), []);
    report(suite.flowIds.shutdown);
  } finally {
    try {
      if (client) {
        const result = await client.shutdown(5_000);
        if (!result.complete) throw new Error("TypeScript Client cleanup did not finish");
      }
    } finally { await harness.close(); }
  }
}
void main().then(runIsolation).then(runTransferStop).catch(error => { console.error(`remote-interop TypeScript failed: ${error instanceof Error ? error.message : "unknown failure"}`); process.exitCode = 1; });
