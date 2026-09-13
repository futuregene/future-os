import { performance } from "node:perf_hooks";
import { SecureChannel } from "../secureChannel";

// Reproducible desktop-JS microbenchmark, not a handset/battery claim. Fixed
// keys are test-only and each case allocates fresh channel state/counters.
test.each([[128, 2000], [1024, 1000], [64 * 1024, 100]])(
  "measures authenticated record roundtrips for %i-byte payloads",
  (size, count) => {
    const a = new Uint8Array(32).fill(1), b = new Uint8Array(32).fill(2), id = new Uint8Array(16).fill(3);
    const sender = new SecureChannel(a, b, id), receiver = new SecureChannel(b, a, id);
    const payload = new Uint8Array(size).fill(42);
    for (let n = 0; n < 100; n++) receiver.open("p.test.evt.session", sender.seal("p.test.evt.session", payload));
    const samples: number[] = [];
    const started = performance.now();
    for (let n = 0; n < count; n++) {
      const before = performance.now();
      const wire = sender.seal("p.test.evt.session", payload);
      const decoded = receiver.open("p.test.evt.session", wire);
      samples.push(performance.now() - before);
      if (decoded.length !== size || decoded[0] !== 42 || wire.length !== size + 44) throw new Error("record corruption");
    }
    const elapsed = performance.now() - started;
    samples.sort((x, y) => x - y);
    console.warn("remote-e2ee-record-benchmark", JSON.stringify({ runtime: process.version, size, count, overheadBytes: 44,
      p50Ms: samples[Math.floor(count * .5)], p95Ms: samples[Math.floor(count * .95)], elapsedMs: elapsed }));
    sender.destroy(); receiver.destroy();
  },
);
