#!/usr/bin/env node
/**
 * The CPU half of the encoding trade, on the real payloads.
 *
 * The phone must JSON.parse the payload in every scheme, so the only question is
 * what replaces what:
 *
 *   today       receive base64 → decode → parse
 *   gzip(b64)   receive gzip → gunzip → decode → parse   (adds gunzip)
 *   gzip+raw    receive gzip → gunzip → parse            (gunzip replaces decode)
 *
 * So `gunzip` versus `base64 decode` is the whole comparison, and JSON.parse is
 * reported alongside to show how much of the "about one second" in the repo's
 * comment is decode work that gzip does not remove.
 *
 * Run with V8 (node) as a proxy: the phone runs Hermes, which is slower, so
 * treat these as relative costs, not device timings.
 */
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { gzipSync, gunzipSync } from "node:zlib";
import { performance } from "node:perf_hooks";

const SAMPLES = [
  ["20260917-133517-bd73ad", "run-20260917-134813-f3297a", "top1 snapshot"],
  ["20260919-184851-1fa911", "run-20260919-230749-30fade", "top3 snapshot"],
  ["20260914-151223-ea0b27", "run-20260914-154247-5a9b8e", "tools snapshot"],
];

const PROTO = fileURLToPath(new URL("../packages/rpc/proto", import.meta.url));

function grpcurl(command) {
  const out = execFileSync("grpcurl", [
    "-plaintext", "-max-msg-sz", "67108864",
    "-import-path", PROTO,
    "-proto", "future.proto", "-d", JSON.stringify(command),
    `unix://${process.env.HOME}/.future/run/agent.sock`,
    "proto.FutureAgent/ExecuteCommand",
  ], { maxBuffer: 256 * 1024 * 1024 }).toString();
  return JSON.parse(out);
}

function snapshot(session, run) {
  const parsed = grpcurl({ id: "s", type: "get_run_snapshot", session_id: session, run_id: run });
  const data = parsed.payload ?? JSON.parse(parsed.data);
  return Buffer.from(JSON.stringify(data.projection), "utf8");
}

/** Median of `runs` calls. */
function time(runs, body) {
  const samples = [];
  for (let index = 0; index < runs; index++) {
    const started = performance.now();
    body();
    samples.push(performance.now() - started);
  }
  samples.sort((a, b) => a - b);
  return samples[Math.floor(samples.length / 2)];
}

const kb = value => `${(value / 1024).toFixed(0)} KB`;
const ms = value => `${value.toFixed(1)} ms`;

console.log(`${"payload".padEnd(16)}${"raw".padStart(9)}${"base64".padStart(9)}${"gzip(b64)".padStart(11)}${"gzip(raw)".padStart(10)}`);
const results = [];
for (const [session, run, label] of SAMPLES) {
  let payload;
  try {
    payload = snapshot(session, run);
  } catch (error) {
    console.log(`${label.padEnd(16)}  skipped: ${String(error.message).slice(0, 50)}`);
    continue;
  }
  const b64 = Buffer.from(payload.toString("base64url"), "utf8");
  const gzB64 = gzipSync(b64, { level: 6 });
  const gzRaw = gzipSync(payload, { level: 6 });
  console.log(`${label.padEnd(16)}${kb(payload.length).padStart(9)}${kb(b64.length).padStart(9)}` +
    `${kb(gzB64.length).padStart(11)}${kb(gzRaw.length).padStart(10)}`);
  results.push({ label, payload, b64, gzB64, gzRaw });
}

console.log(`\nCPU, median of 9 (V8 proxy — the phone's Hermes is slower):`);
console.log("  " + "payload".padEnd(16) + "b64decode".padStart(11) + "gunzip(b64)".padStart(13) +
  "gunzip(raw)".padStart(12) + "JSON.parse".padStart(12) + "  net of gunzip vs decode");
for (const { label, payload, b64, gzB64, gzRaw } of results) {
  const decode = time(9, () => Buffer.from(b64.toString("utf8"), "base64"));
  const gunzipB64 = time(9, () => gunzipSync(gzB64));
  const gunzipRaw = time(9, () => gunzipSync(gzRaw));
  const parse = time(9, () => JSON.parse(payload.toString("utf8")));
  const verdict = gunzipRaw <= decode ? "gunzip CHEAPER than decode" : `gunzip +${(gunzipRaw - decode).toFixed(1)} ms`;
  console.log("  " + label.padEnd(16) + ms(decode).padStart(11) + ms(gunzipB64).padStart(13) +
    ms(gunzipRaw).padStart(12) + ms(parse).padStart(12) + "  " + verdict);
}
console.log("\nJSON.parse is paid in every scheme and dominates; gzip only trades decode for gunzip.");
