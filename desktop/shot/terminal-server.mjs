/**
 * Stand-in for the desktop's loopback PTY server
 * (`desktop/src-tauri/src/terminal/server`), for the screenshot harness.
 *
 * Speaks the same control routes and reuses the real client's frame shape, and
 * replays a canned shell session over the WebSocket, so the embedded terminal
 * panel can be captured without spawning a real shell.
 *
 * Usage: node desktop/shot/terminal-server.mjs [port]
 *   SHOT_TERMINAL_CWD  directory the prompt claims (display only)
 */
import { createServer } from "node:http";
import { WebSocketServer } from "ws";

const PORT = Number(process.argv[2] ?? 7391);
const CWD = process.env.SHOT_TERMINAL_CWD ?? "/Users/lixin/Research/dopamine-decision";

/** A shell transcript: listing, test run and git status of the demo project. */
const TRANSCRIPT = [
  "\u001b[1;32m➜\u001b[0m \u001b[1;36mdopamine-decision\u001b[0m git:(\u001b[31mmain\u001b[0m) ",
  "ls\r\n",
  "\u001b[1;34mdata\u001b[0m    \u001b[1;34mfigures\u001b[0m \u001b[1;34mnotes\u001b[0m    \u001b[1;34mpapers\u001b[0m  draft-v3.md  README.md\r\n",
  "\u001b[1;32m➜\u001b[0m \u001b[1;36mdopamine-decision\u001b[0m git:(\u001b[31mmain\u001b[0m) ",
  "python3 -m pytest tests/ -q\r\n",
  "\u001b[32m....................\u001b[0m                                     \u001b[32m[ 62%]\u001b[0m\r\n",
  "\u001b[32m............\u001b[0m                                         \u001b[32m[100%]\u001b[0m\r\n",
  "32 passed, 2 warnings in 4.81s\r\n",
  "\u001b[1;32m➜\u001b[0m \u001b[1;36mdopamine-decision\u001b[0m git:(\u001b[31mmain\u001b[0m) ",
  "git status --short\r\n",
  " \u001b[31mM\u001b[0m notes/compare.md\r\n",
  "\u001b[31m??\u001b[0m figures/forest-plot.png\r\n",
  "\u001b[1;32m➜\u001b[0m \u001b[1;36mdopamine-decision\u001b[0m git:(\u001b[31mmain\u001b[0m) ",
].join("");

const sessions = new Map();
let nextId = 1;

const CORS = {
  "content-type": "application/json",
  "access-control-allow-origin": "*",
  "access-control-allow-headers": "authorization,content-type",
  "access-control-allow-methods": "GET,POST,PATCH,DELETE,OPTIONS",
};

function json(response, status, body) {
  response.writeHead(status, CORS);
  response.end(JSON.stringify(body));
}

function info(id, threadId, cols, rows) {
  return {
    id,
    threadId,
    title: "zsh",
    command: "/bin/zsh",
    args: [],
    cwd: CWD,
    status: "running",
    exitCode: null,
    pid: 48213,
    cols,
    rows,
  };
}

const server = createServer((request, response) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${PORT}`);
  if (request.method === "OPTIONS") {
    response.writeHead(204, CORS);
    response.end();
    return;
  }
  if (url.pathname === "/terminal" && request.method === "GET") {
    const threadId = url.searchParams.get("threadId") ?? "";
    json(response, 200, [...sessions.values()].filter(session => session.threadId === threadId));
    return;
  }
  if (url.pathname === "/terminal" && request.method === "POST") {
    let body = "";
    request.on("data", chunk => (body += chunk));
    request.on("end", () => {
      const input = JSON.parse(body || "{}");
      const id = `term_${nextId++}`;
      const session = info(id, input.threadId ?? "", input.cols ?? 100, input.rows ?? 28);
      sessions.set(id, session);
      console.log("[terminal-server] created", id, "cwd", session.cwd);
      json(response, 200, session);
    });
    return;
  }
  if (/^\/terminal\/[^/]+\/connect-token$/.test(url.pathname) && request.method === "POST") {
    json(response, 200, { ticket: "shot-ticket", expiresIn: 60 });
    return;
  }
  if (/^\/terminal\/[^/]+$/.test(url.pathname) && request.method === "GET") {
    const session = sessions.get(url.pathname.split("/")[2]);
    if (!session) {
      json(response, 404, { error: { code: "NOT_FOUND", message: "no such terminal" } });
      return;
    }
    json(response, 200, session);
    return;
  }
  json(response, 404, { error: { code: "NOT_FOUND", message: "unknown route" } });
});

const wss = new WebSocketServer({ server });
wss.on("connection", (socket) => {
  // Replay the transcript, then echo typed input so the panel feels live.
  socket.send(TRANSCRIPT, { binary: false });
  socket.on("message", (data) => {
    const text = String(data);
    if (/^[\x20-\x7e\r\n]+$/.test(text))
      socket.send(text.replace(/\r/g, "\r\n"), { binary: false });
  });
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`[terminal-server] http://127.0.0.1:${PORT}`);
});
