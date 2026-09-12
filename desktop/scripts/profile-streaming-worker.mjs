// Production-worker compute/heap benchmark. Each invocation is a fresh process:
// node --expose-gc scripts/profile-streaming-worker.mjs list baseline
// node --expose-gc scripts/profile-streaming-worker.mjs mixed baseline
// No Agent, desktop deployment, browser, filesystem data or network is involved.
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import { performance } from "node:perf_hooks";
import { resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { runInNewContext } from "node:vm";

const fixture = process.argv[2] ?? "list";
const label = process.argv[3] ?? "candidate";
assert.ok(global.gc, "Run Node with --expose-gc to measure retained heap");
assert.ok(["list", "mixed", "code"].includes(fixture));
const assets = process.argv[4]
  ? pathToFileURL(`${resolve(process.argv[4])}${sep}`)
  : new URL("../dist/assets/", import.meta.url);
const workers = (await readdir(assets)).filter(name => /^streamingMarkdown\.worker-.*\.js$/.test(name));
assert.equal(workers.length, 1);
const bundle = await readFile(new URL(workers[0], assets), "utf8");
let lastResponse;
const scope = { postMessage: message => { lastResponse = message; } };
runInNewContext(bundle, scope, { timeout: 10000 });
const updates = 80;
let text = fixture === "list"
  ? Array.from({ length: 800 }, (_, i) => `- Item ${i}: **value** with \`code\` and [link](https://example.com/${i}).`).join("\n")
  : fixture === "mixed"
    ? Array.from({ length: 120 }, (_, i) => `## Section ${i}\n\nParagraph ${i} with **bold** and $x^2$.\n\n- Item ${i}\n\n\`\`\`ts\nconst n = ${i};\n\`\`\`\n\n`).join("") + "Growing tail"
    : "```text\n" + "long code line with a sample value\n".repeat(1500);
const initialChars = text.length;
let id = 0;
function project(live) {
  const cpuStart = process.cpuUsage();
  const start = performance.now();
  scope.onmessage({ data: { id: ++id, text, live } });
  const duration = performance.now() - start;
  const cpu = process.cpuUsage(cpuStart);
  assert.equal(lastResponse.text, text);
  assert.equal(lastResponse.blocks.map(block => block.content).join(""), text);
  assert.equal(lastResponse.blocks.at(-1)?.live, live);
  return { ms: duration, cpuMs: (cpu.user + cpu.system) / 1000 };
}
function heap() { return process.memoryUsage().heapUsed; }
function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length / 2)];
}
global.gc();
const baseHeap = heap();
const first = project(true);
let highWater = heap();
const appendMs = [];
const appendCpuMs = [];
for (let i = 0; i < updates; i++) {
  text += fixture === "list" ? `\n- Added ${i}: **value**` : ` append-${i}`;
  const sample = project(true);
  appendMs.push(sample.ms);
  appendCpuMs.push(sample.cpuMs);
  highWater = Math.max(highWater, heap());
}
global.gc();
const retainedAfterAppends = heap();
const unchangedTextMs = [];
const unchangedTextCpuMs = [];
for (let i = 0; i < 10; i++) {
  const sample = project(i % 2 !== 0);
  unchangedTextMs.push(sample.ms);
  unchangedTextCpuMs.push(sample.cpuMs);
}
global.gc();
const result = {
  label, fixture, node: process.version, platform: process.platform,
  workerBundle: workers[0], initialChars, updates,
  firstParseMs: first.ms, firstParseCpuMs: first.cpuMs,
  appendMedianMs: median(appendMs), appendMaxMs: Math.max(...appendMs), appendMedianCpuMs: median(appendCpuMs),
  unchangedTextMedianMs: median(unchangedTextMs), unchangedTextMedianCpuMs: median(unchangedTextCpuMs),
  retainedAfterAppendsMiB: (retainedAfterAppends - baseHeap) / 1024 ** 2,
  sampledHeapHighWaterMiB: (highWater - baseHeap) / 1024 ** 2,
  appendMs, unchangedTextMs, appendCpuMs, unchangedTextCpuMs,
};
console.log(JSON.stringify(result));
