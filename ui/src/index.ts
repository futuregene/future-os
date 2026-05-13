/**
 * xihu TypeScript TUI entry point.
 * Usage: node dist/index.js [--socket <path>] [--port <port>]
 */

import { App } from "./app.js";

const args = process.argv.slice(2);
let serverUrl = "http://localhost:7890";

for (let i = 0; i < args.length; i++) {
  if (args[i] === "--socket" && i + 1 < args.length) {
    const path = args[i + 1];
    // Unix socket - use http://localhost with no actual host
    // The RPC client needs to support unix sockets via fetch
    serverUrl = `http://localhost/`;
    process.env.XIHU_SOCKET = path;
    i++;
  } else if (args[i] === "--port" && i + 1 < args.length) {
    serverUrl = `http://localhost:${args[i + 1]}/`;
    i++;
  } else if (args[i] === "--url" && i + 1 < args.length) {
    serverUrl = args[i + 1];
    i++;
  }
}

console.error("Starting xihu TUI...");
console.error(`Server: ${serverUrl}`);

const app = new App(serverUrl);

process.on("SIGINT", async () => {
  await app.stop();
  process.exit(0);
});

process.on("SIGTERM", async () => {
  await app.stop();
  process.exit(0);
});

try {
  await app.start();
} catch (err) {
  console.error("Failed to start:", err);
  process.exit(1);
}
