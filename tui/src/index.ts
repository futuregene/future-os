/**
 * xihu TypeScript TUI entry point.
 * Usage: node dist/index.js [--socket <path>] [--port <port>] [--url <url>]
 *
 * Examples:
 *   node dist/index.js --socket /tmp/xihu.sock
 *   node dist/index.js --port 7890
 */

import { App } from "./app.js";
import { RpcClient } from "./rpc/client.js";

const args = process.argv.slice(2);
let serverUrl = "http://localhost:7890";
let socketPath: string | undefined;

for (let i = 0; i < args.length; i++) {
  if (args[i] === "--socket" && i + 1 < args.length) {
    socketPath = args[i + 1];
    process.env.XIHU_SOCKET = socketPath;
    i++;
  } else if (args[i] === "--port" && i + 1 < args.length) {
    serverUrl = `http://localhost:${args[i + 1]}/`;
    i++;
  } else if (args[i] === "--url" && i + 1 < args.length) {
    serverUrl = args[i + 1];
    if (!serverUrl.endsWith("/")) serverUrl += "/";
    i++;
  }
}

const app = new App(serverUrl);

process.on("SIGINT", async () => {
  await app.stop();
  process.exit(0);
});

process.on("SIGTERM", async () => {
  await app.stop();
  process.exit(0);
});

app.start().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
