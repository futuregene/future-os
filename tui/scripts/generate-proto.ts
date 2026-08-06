/**
 * Reads proto/future.proto and embeds its content into the CLI's
 * grpc-client.ts (its EMBEDDED_PROTO constant). The TUI no longer embeds the
 * proto — it loads it from the shared @future-os/rpc package, whose own
 * EMBEDDED_PROTO is refreshed by shared/future-rpc/scripts/generate-proto.ts.
 * Once the CLI also adopts @future-os/rpc this script is deleted.
 *
 * Run: bun run scripts/generate-proto.ts
 */
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const protoPath = path.resolve(__dirname, "..", "..", "proto", "future.proto");
const clientPaths = [
  path.resolve(__dirname, "..", "..", "cli", "src", "rpc", "grpc-client.ts"),
];

const proto = fs.readFileSync(protoPath, "utf-8")
  .replace(/\\/g, "\\\\")
  .replace(/`/g, "\\`")
  .replace(/\$\{/g, "\\${");

// Find the EMBEDDED_PROTO constant and replace its content between the backticks
const endMarker = "`;";
for (const clientPath of clientPaths) {
  let client = fs.readFileSync(clientPath, "utf-8");
  const startMarker = client.includes("export const EMBEDDED_PROTO = `")
    ? "export const EMBEDDED_PROTO = `"
    : "const EMBEDDED_PROTO = `";
  const startIdx = client.indexOf(startMarker);
  const endIdx = client.indexOf(endMarker, startIdx + startMarker.length);

  if (startIdx === -1 || endIdx === -1) {
    console.error(`Could not find EMBEDDED_PROTO in ${clientPath}`);
    process.exit(1);
  }

  client =
    client.slice(0, startIdx + startMarker.length) +
    proto +
    client.slice(endIdx);
  fs.writeFileSync(clientPath, client);
  console.log(`Embedded proto updated in ${clientPath}`);
}
