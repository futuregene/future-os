import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import type { RemoteClient } from "@future-os/remote-client/client";
import { downloadPrepared, prepareDownload } from "@future-os/remote-client/transfer";
import { suite, transfer } from "./contract";
import { bounded, Harness, until } from "./harness";
import { controller } from "./isolation";

export async function runTransferStop(): Promise<void> {
  for (const scenario of transfer.cases) {
    const { harness, ready } = await Harness.start();
    let client: RemoteClient | undefined;
    let owner: ReturnType<typeof controller> | undefined;
    let delivered = 0, hashes = 0;
    const file = { bytes: [] as number[], closed: false, removed: false };
    try {
      owner = controller(ready);
      const active = owner.create();
      client = active.client;
      await client.open(); await bounded(active.connected, 20_000);
      const info = await prepareDownload(client, suite.event.sessionId, { path: "fixture.bin", name: "fixture.bin" }, { verifiedCached: async () => null }, "original");
      await harness.command("hold_chunk");
      const outcome = downloadPrepared(client, info, {
        maxFileBytes: 10 * 1024 * 1024, verifiedCached: async () => null,
        createDownload: () => ({ file, handle: { writeBytes: bytes => { delivered += bytes.length; file.bytes.push(...bytes); }, close: () => { file.closed = true; } } }),
        size: output => output.bytes.length,
        remove: output => { output.removed = true; output.bytes = []; },
        hash: async output => { hashes++; return createHash("sha256").update(Uint8Array.from(output.bytes)).digest("hex"); },
      }).then(() => ({ completed: true }), () => ({ completed: false }));
      await until(async () => ((await harness.command("metrics")).metrics as { chunkWaiting: boolean }).chunkWaiting);
      const attempts = owner.connectionAttempts();
      assert.equal((await client.shutdown(1_000)).complete, true);
      assert.equal((await bounded(outcome, 1_000)).completed, false);
      assert.equal(((await harness.command("metrics")).metrics as { chunkWaiting: boolean }).chunkWaiting, true);
      assert.equal(delivered, scenario.expectedDeliveredBytes);
      assert.equal(file.closed && file.removed, true);
      assert.equal(hashes, 0);
      await harness.command("release_chunks");
      await until(async () => ((await harness.command("metrics")).metrics as { chunkFinished: number }).chunkFinished > 0);
      await client.open(); // A stopped role must not create a replacement.
      assert.equal(owner.connectionAttempts() - attempts, scenario.expectedRestarts);
      assert.equal((await client.waitTasks(0)).complete, true);
      assert.equal(delivered, scenario.expectedDeliveredBytes);
      assert.deepEqual(owner.errors.map(error => error.message), []);
    } finally {
      // Release synchronous host IO before awaiting Server teardown, including
      // errors/timeouts on the Client side. IPC contains no logged credentials.
      try { await harness.command("release_chunks"); } catch { /* host cleanup below remains authoritative */ }
      try {
        if (client) assert.equal((await client.shutdown(5_000)).complete, true);
      } finally { await harness.close(); }
    }
    assert.equal(delivered, scenario.expectedDeliveredBytes);
    assert.equal(hashes, 0);
    assert.equal(owner?.errors.length, 0);
    console.log(`remote-interop typescript ${scenario.id} PASS`);
  }
}
