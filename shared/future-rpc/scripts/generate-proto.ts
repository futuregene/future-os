/**
 * Reads proto/future.proto and embeds its content into this package's
 * src/proto.ts (the EMBEDDED_PROTO constant) — the single source the TUI and
 * CLI consume once they adopt @future-os/rpc.
 *
 * Run: bun run scripts/generate-proto.ts   (or `make generate-proto`)
 */
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
// shared/future-rpc/scripts → repo root is two levels up.
const protoPath = path.resolve(__dirname, "..", "..", "..", "proto", "future.proto");
const target = path.resolve(__dirname, "..", "src", "proto.ts");

const proto = fs
  .readFileSync(protoPath, "utf-8")
  .replace(/\\/g, "\\\\")
  .replace(/`/g, "\\`")
  .replace(/\$\{/g, "\\${");

let client = fs.readFileSync(target, "utf-8");
const startMarker = "export const EMBEDDED_PROTO = `";
const startIdx = client.indexOf(startMarker);
const endIdx = client.indexOf("`;", startIdx + startMarker.length);

if (startIdx === -1 || endIdx === -1) {
  console.error(`Could not find EMBEDDED_PROTO in ${target}`);
  process.exit(1);
}

client = client.slice(0, startIdx + startMarker.length) + proto + client.slice(endIdx);
fs.writeFileSync(target, client);
console.log(`Embedded proto updated in ${target}`);
