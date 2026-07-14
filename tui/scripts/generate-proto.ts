/**
 * Reads proto/future.proto and embeds its content into
 * src/rpc/grpc-client.ts (the EMBEDDED_PROTO constant).
 *
 * Run: bun run scripts/generate-proto.ts
 */
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const protoPath = path.resolve(__dirname, "..", "..", "proto", "future.proto");
const clientPath = path.resolve(__dirname, "..", "src", "rpc", "grpc-client.ts");

const proto = fs.readFileSync(protoPath, "utf-8")
  .replace(/`/g, "\\`")
  .replace(/\$\{/g, "\\${");

let client = fs.readFileSync(clientPath, "utf-8");

// Find the EMBEDDED_PROTO constant and replace its content between the backticks
const startMarker = "export const EMBEDDED_PROTO = `";
const endMarker = "`;";

const startIdx = client.indexOf(startMarker);
const endIdx = client.indexOf(endMarker, startIdx + startMarker.length);

if (startIdx === -1 || endIdx === -1) {
  console.error("Could not find EMBEDDED_PROTO in grpc-client.ts");
  process.exit(1);
}

const newContent =
  client.slice(0, startIdx + startMarker.length) +
  proto +
  client.slice(endIdx);

fs.writeFileSync(clientPath, newContent);
console.log(`Embedded proto updated from ${protoPath}`);
