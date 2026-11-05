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
import process from "node:process";
import { WebSocketServer } from "ws";

const PORT = Number(process.argv[2] ?? 7391);
const CWD = process.env.SHOT_TERMINAL_CWD ?? "/Users/lixin/Research/dopamine-decision";

/** A shell transcript: listing, test run and git status of the demo project. */
const TRANSCRIPT = [
  "\u001B[1;32m➜\u001B[0m \u001B[1;36mdopamine-decision\u001B[0m git:(\u001B[31mmain\u001B[0m) ",
  "ls\r\n",
  "\u001B[1;34mdata\u001B[0m    \u001B[1;34mfigures\u001B[0m \u001B[1;34mnotes\u001B[0m    \u001B[1;34mpapers\u001B[0m  draft-v3.md  README.md\r\n",
  "\u001B[1;32m➜\u001B[0m \u001B[1;36mdopamine-decision\u001B[0m git:(\u001B[31mmain\u001B[0m) ",
  "python3 -m pytest tests/ -q\r\n",
  "\u001B[32m....................\u001B[0m                                     \u001B[32m[ 62%]\u001B[0m\r\n",
  "\u001B[32m............\u001B[0m                                         \u001B[32m[100%]\u001B[0m\r\n",
  "32 passed, 2 warnings in 4.81s\r\n",
  "\u001B[1;32m➜\u001B[0m \u001B[1;36mdopamine-decision\u001B[0m git:(\u001B[31mmain\u001B[0m) ",
  "git status --short\r\n",
  " \u001B[31mM\u001B[0m notes/compare.md\r\n",
  "\u001B[31m??\u001B[0m figures/forest-plot.png\r\n",
  "\u001B[1;32m➜\u001B[0m \u001B[1;36mdopamine-decision\u001B[0m git:(\u001B[31mmain\u001B[0m) ",
].join("");

const sessions = new Map();
let nextId = 1;

/**
 * Commands the canned shell answers for real.
 *
 * A transcript that fits on screen says nothing about scrolling, and "the
 * terminal is cut off and will not scroll to the end" is exactly a property of
 * the screen *after* it has overflowed — so one command has to produce more
 * lines than the panel can hold. Only commands whose answer is fixed live here;
 * anything else is echoed back as typed, as before.
 */
const PROMPT = "\u001B[1;32m➜\u001B[0m \u001B[1;36mdopamine-decision\u001B[0m git:(\u001B[31mmain\u001B[0m) ";
const COMMANDS = [
  {
    pattern: /^seq 1 (\d+)$/,
    answer: (_line, count) => {
      const total = Math.min(Number(count) || 0, 400);
      let out = "";
      for (let index = 1; index <= total; index += 1)
        out += `${index}\r\n`;
      return out;
    },
  },
];

/** What the shell prints for one submitted line: the echo, then any answer. */
function reply(line) {
  const trimmed = line.trim();
  for (const command of COMMANDS) {
    const match = command.pattern.exec(trimmed);
    if (match)
      return `${PROMPT}${trimmed}\r\n${command.answer(trimmed, match[1])}`;
  }
  return `${PROMPT}${line}\r\n`;
}

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
      process.stdout.write(`[terminal-server] created ${id} cwd ${session.cwd}\n`);
      json(response, 200, session);
    });
    return;
  }
  if (/^\/terminal\/[^/]+\/connect-token$/.test(url.pathname) && request.method === "POST") {
    json(response, 200, { ticket: "shot-ticket", expiresIn: 60 });
    return;
  }
  if (/^\/terminal\/[^/]+$/.test(url.pathname) && request.method === "PATCH") {
    const session = sessions.get(url.pathname.split("/")[2]);
    if (!session) {
      json(response, 404, { error: { code: "NOT_FOUND", message: "no such terminal" } });
      return;
    }
    // Resize: the client sends cols/rows; the canned transcript does not reflow,
    // but the request must succeed or the panel shows a connection error.
    let body = "";
    request.on("data", (chunk) => (body += chunk));
    request.on("end", () => {
      try {
        const patch = JSON.parse(body || "{}");
        if (Number.isFinite(patch.cols)) session.cols = Number(patch.cols);
        if (Number.isFinite(patch.rows)) session.rows = Number(patch.rows);
      }
      catch {}
      json(response, 200, session);
    });
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
  // Replay the transcript, then answer typed lines so the panel feels live.
  socket.send(TRANSCRIPT, { binary: false });
  // Input arrives as it is typed: a whole pasted word in one frame, the newline
  // (or Enter as `\r`) in the next. Buffer until a line is submitted, so a
  // command typed key by key and one inserted at once behave the same.
  let pending = "";
  socket.on("message", (data) => {
    const text = String(data);
    if (!/^[\x20-\x7E\r\n]+$/.test(text))
      return;
    const lines = (pending + text).split(/\r\n|\r|\n/);
    pending = lines.pop() ?? "";
    if (lines.length === 0)
      return;
    socket.send(lines.map(reply).join(""), { binary: false });
  });
});

server.listen(PORT, "127.0.0.1", () => {
  process.stdout.write(`[terminal-server] http://127.0.0.1:${PORT}\n`);
});
