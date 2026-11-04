/**
 * A small Chrome DevTools Protocol client for the UI harness.
 *
 * The screenshot harness (`scripts/screenshots/cdp.mjs`) is a CLI built around
 * one scenario per process. A test run needs something different: one browser,
 * many cases, an event stream per case (console, network, performance), and a
 * screenshot after every step. So this module is a library, not a CLI, and it
 * keeps the same interaction rules as the screenshot harness — real mouse
 * events, accessibility-first element matching — so the two cannot drift into
 * "clicking" two different things.
 */
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import WebSocket from "ws";

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

export function chromePath() {
  const candidates = [
    process.env.CHROME_PATH,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
  ].filter(Boolean);
  const found = candidates.find(path => existsSync(path));
  if (!found)
    throw new Error("Chrome not found. Set CHROME_PATH to a Chrome/Chromium binary.");
  return found;
}

/** Element matching, ranked — same rules as `scripts/screenshots/cdp.mjs`. */
const MATCH_FN = `
function __harnessMatch(wanted, root) {
  const INTERACTIVE = 'button,[role=button],[role=menuitem],[role=tab],[role=switch],[role=checkbox],a[href],input,textarea';
  const firstLine = el => ((el.innerText || '').trim().split('\\n')[0] || '');
  const candidates = [];
  for (const el of (root || document).querySelectorAll('[aria-label]')) {
    const label = el.getAttribute('aria-label') || '';
    if (label === wanted) candidates.push({ el, score: 0 });
    else if (label.includes(wanted)) candidates.push({ el, score: 3 });
  }
  for (const el of (root || document).querySelectorAll(INTERACTIVE)) {
    const text = firstLine(el);
    if (text === wanted) candidates.push({ el, score: 1 });
    else if (text.startsWith(wanted)) candidates.push({ el, score: 2 });
  }
  if (!candidates.length) return null;
  candidates.sort((a, b) => a.score - b.score);
  return candidates[0].el;
}
`;

const LOCATE = (query, byText, selector) => `(() => {
  ${MATCH_FN}
  const wanted = ${JSON.stringify(query ?? "")};
  let target = null;
  if (${JSON.stringify(Boolean(selector))}) {
    const nodes = [...document.querySelectorAll(${JSON.stringify(selector ?? "")})];
    target = ${JSON.stringify(Boolean(query))}
      ? (nodes.find(el => ((el.innerText || '').trim().split('\\n')[0] || '').startsWith(wanted)) ?? nodes[0] ?? null)
      : (nodes[0] ?? null);
  } else if (${JSON.stringify(Boolean(byText))}) {
    const matches = [...document.querySelectorAll('div,span,button,p,a')]
      .filter(el => ((el.innerText || '').trim().split('\\n')[0] || '').startsWith(wanted));
    target = matches[matches.length - 1] || null;
  } else {
    target = __harnessMatch(wanted);
  }
  if (!target) return null;
  target.scrollIntoView({ block: 'center' });
  const rect = target.getBoundingClientRect();
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2,
           w: rect.width, h: rect.height,
           label: target.getAttribute('aria-label') || (target.innerText || '').slice(0, 40) };
})()`;

/** One websocket to the browser; page and browser commands share it. */
class Connection {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = new Map();
    ws.on("message", raw => this.handle(raw));
  }

  send(method, params = {}, sessionId = undefined) {
    const id = this.nextId++;
    const message = { id, method, params };
    if (sessionId)
      message.sessionId = sessionId;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify(message));
    });
  }

  on(method, callback, sessionId = null) {
    const key = `${sessionId ?? "*"}|${method}`;
    const list = this.listeners.get(key) ?? [];
    list.push(callback);
    this.listeners.set(key, list);
  }

  handle(raw) {
    const message = JSON.parse(raw.toString());
    if (message.id && this.pending.has(message.id)) {
      const { resolve, reject } = this.pending.get(message.id);
      this.pending.delete(message.id);
      if (message.error)
        reject(new Error(message.error.message));
      else resolve(message.result);
      return;
    }
    const wildcard = `*|${message.method}`;
    const exact = `${message.sessionId ?? "*"}|${message.method}`;
    for (const listener of this.listeners.get(wildcard) ?? [])
      listener(message.params, message);
    if (exact !== wildcard) {
      for (const listener of this.listeners.get(exact) ?? [])
        listener(message.params, message);
    }
  }
}

export class CdpPage {
  constructor(connection, sessionId, { process: chromeProcess } = {}) {
    this.connection = connection;
    this.sessionId = sessionId;
    this.process = chromeProcess;
    this.viewport = { width: 1280, height: 860, scale: 2 };
    this.framesDir = null;
    this.frames = [];
    this.frameSeq = 0;
    this.consoleMessages = [];
    this.exceptions = [];
    this.networkFailures = [];
    this.networkRequests = new Map();
    this.onEvent = null;
  }

  static async launch({ width = 1280, height = 860, scale = 2, userDataDir } = {}) {
    const profile = userDataDir ?? join(process.env.TMPDIR ?? "/tmp", `future-harness-${Date.now()}`);
    mkdirSync(profile, { recursive: true });
    const chrome = spawn(chromePath(), [
      "--headless=new",
      "--remote-debugging-port=0",
      `--user-data-dir=${profile}`,
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-background-networking",
      "--disable-component-update",
      "--disable-extensions",
      "--disable-sync",
      "--hide-scrollbars",
      "--mute-audio",
      "--no-sandbox",
      "--force-device-scale-factor=1",
      `--window-size=${width},${height}`,
      "about:blank",
    ], { stdio: ["ignore", "ignore", "pipe"] });

    let stderr = "";
    chrome.stderr.on("data", (chunk) => {
      stderr += chunk;
    });

    const portFile = join(profile, "DevToolsActivePort");
    let port = null;
    let browserPath = null;
    for (let i = 0; i < 120; i += 1) {
      if (existsSync(portFile)) {
        const [p, path] = readFileSync(portFile, "utf8").trim().split("\n");
        if (p && path) {
          port = Number(p);
          browserPath = path;
          break;
        }
      }
      await sleep(100);
    }
    if (!port)
      throw new Error(`Chrome did not open a debugging port.\n${stderr}`);

    const ws = new WebSocket(`ws://127.0.0.1:${port}${browserPath}`);
    await new Promise((resolve, reject) => {
      ws.once("open", resolve);
      ws.once("error", reject);
    });
    const connection = new Connection(ws);
    const { targetId } = await connection.send("Target.createTarget", { url: "about:blank" });
    const { sessionId } = await connection.send("Target.attachToTarget", { targetId, flatten: true });
    const page = new CdpPage(connection, sessionId, { process: chrome });
    page.browserPort = port;
    page.targetId = targetId;
    await page.send("Page.enable");
    await page.send("Runtime.enable");
    await page.send("Log.enable");
    await page.send("Network.enable");
    await page.send("Performance.enable");
    await page.send("Emulation.setDeviceMetricsOverride", {
      width,
      height,
      deviceScaleFactor: scale,
      mobile: false,
    });
    page.viewport = { width, height, scale };
    page.installEventHandlers();
    return page;
  }

  send(method, params = {}) {
    return this.connection.send(method, params, this.sessionId);
  }

  on(method, callback) {
    this.connection.on(method, callback, this.sessionId);
  }

  installEventHandlers() {
    this.on("Runtime.consoleAPICalled", (params) => {
      const text = (params.args ?? []).map(arg => arg.value ?? arg.description ?? arg.type).join(" ").slice(0, 2000);
      const entry = { ts: Date.now(), type: params.type, text };
      this.consoleMessages.push(entry);
      this.emitObservation({ channel: "console", ...entry });
    });
    this.on("Runtime.exceptionThrown", (params) => {
      const details = params.exceptionDetails ?? {};
      const entry = {
        ts: Date.now(),
        text: details.exception?.description ?? details.text ?? "exception",
        url: details.url ?? null,
        line: details.lineNumber ?? null,
        column: details.columnNumber ?? null,
      };
      this.exceptions.push(entry);
      this.emitObservation({ channel: "exception", ...entry });
    });
    this.on("Log.entryAdded", (params) => {
      const entry = params.entry ?? {};
      if (entry.level === "error" || entry.level === "warning")
        this.emitObservation({ channel: "log", ts: Date.now(), level: entry.level, text: `${entry.source}: ${entry.text}`.slice(0, 2000) });
    });
    this.on("Network.requestWillBeSent", (params) => {
      this.networkRequests.set(params.requestId, {
        url: params.request?.url,
        method: params.request?.method,
        type: params.type,
        start: Date.now(),
      });
    });
    this.on("Network.responseReceived", (params) => {
      const request = this.networkRequests.get(params.requestId);
      if (!request)
        return;
      request.status = params.response?.status;
      request.end = Date.now();
      if (params.response?.status >= 400) {
        this.networkFailures.push({ ...request, error: `HTTP ${params.response.status}` });
        this.emitObservation({ channel: "network", ...request, error: `HTTP ${params.response.status}` });
      }
    });
    this.on("Network.loadingFailed", (params) => {
      const request = this.networkRequests.get(params.requestId) ?? { url: "?" };
      if (params.canceled)
        return;
      const failure = { ...request, end: Date.now(), error: params.errorText };
      this.networkFailures.push(failure);
      this.emitObservation({ channel: "network", ...failure });
    });
    this.on("Page.screencastFrame", (params) => {
      if (!this.framesDir)
        return;
      const file = join(this.framesDir, `frame-${String(this.frameSeq++).padStart(5, "0")}.jpg`);
      writeFileSync(file, Buffer.from(params.data, "base64"));
      this.frames.push({ file, ts: Date.now() });
      this.send("Page.screencastFrameAck", { sessionId: params.sessionId }).catch(() => {});
    });
  }

  emitObservation(event) {
    if (this.onEvent)
      this.onEvent(event);
  }

  async evaluate(expression, { awaitPromise = true } = {}) {
    const response = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise,
      returnByValue: true,
    });
    if (response.exceptionDetails)
      throw new Error(response.exceptionDetails.exception?.description ?? response.exceptionDetails.text ?? "evaluation failed");
    return response.result?.value;
  }

  async navigate(url, { ready = null, timeout = 30_000, settle = 400 } = {}) {
    await this.send("Page.navigate", { url });
    const deadline = Date.now() + timeout;
    let readySince = null;
    while (Date.now() < deadline) {
      try {
        const state = await this.evaluate("document.readyState");
        if (state === "complete") {
          if (!ready) {
            readySince ??= Date.now();
            if (Date.now() - readySince > settle)
              break;
          }
          else if (await this.evaluate(ready).catch(() => false)) {
            break;
          }
        }
      }
      catch {
        // The execution context is replaced while navigating; retry.
      }
      await sleep(150);
    }
    await sleep(settle);
  }

  async locate(target) {
    const query = typeof target === "string" ? target : target?.aria ?? target?.text ?? "";
    const byText = typeof target === "object" && target?.text;
    const selector = typeof target === "object" ? target?.selector : null;
    return this.evaluate(LOCATE(query, byText, selector));
  }

  async click(target, { label = null } = {}) {
    const spot = await this.locate(target);
    if (!spot)
      throw new Error(`target not found: ${label ?? JSON.stringify(target)}`);
    if (spot.w === 0 || spot.h === 0)
      throw new Error(`target is not visible: ${label ?? JSON.stringify(target)}`);
    await this.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: spot.x, y: spot.y, button: "none", buttons: 0 });
    await this.send("Input.dispatchMouseEvent", { type: "mousePressed", x: spot.x, y: spot.y, button: "left", buttons: 1, clickCount: 1 });
    await sleep(60);
    await this.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: spot.x, y: spot.y, button: "left", buttons: 0, clickCount: 1 });
    await sleep(120);
    return spot;
  }

  async hover(target) {
    const spot = await this.locate(target);
    if (!spot)
      throw new Error(`target not found: ${JSON.stringify(target)}`);
    await this.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: spot.x, y: spot.y, button: "none", buttons: 0 });
    // Hover-revealed controls (a row's "…") mount on the next frame.
    await sleep(260);
    return spot;
  }

  async type(text) {
    await this.send("Input.insertText", { text });
  }

  async press(key) {
    const code = key.length === 1 ? `Key${key.toUpperCase()}` : key;
    await this.send("Input.dispatchKeyEvent", { type: "keyDown", key, code, windowsVirtualKeyCode: keyCode(key) });
    await sleep(30);
    await this.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode: keyCode(key) });
    await sleep(80);
  }

  async scroll(dx, dy) {
    await this.send("Input.dispatchMouseEvent", {
      type: "mouseWheel",
      x: this.viewport.width / 2,
      y: this.viewport.height / 2,
      deltaX: dx,
      deltaY: dy,
    });
    await sleep(200);
  }

  async screenshot(path) {
    const { data } = await this.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
    writeFileSync(path, Buffer.from(data, "base64"));
    return path;
  }

  async metrics() {
    const { metrics } = await this.send("Performance.getMetrics");
    const value = name => metrics.find(m => m.name === name)?.value ?? 0;
    return {
      jsHeapUsed: value("JSHeapUsedSize"),
      jsHeapTotal: value("JSHeapTotalSize"),
      nodes: value("Nodes"),
      documents: value("Documents"),
      jsEventListeners: value("JSEventListeners"),
      layoutCount: value("LayoutCount"),
      recalcStyleCount: value("RecalcStyleCount"),
      taskDuration: value("TaskDuration"),
      scriptDuration: value("ScriptDuration"),
    };
  }

  async startScreencast(framesDir) {
    this.framesDir = framesDir;
    this.frames = [];
    this.frameSeq = 0;
    mkdirSync(framesDir, { recursive: true });
    await this.send("Page.startScreencast", {
      format: "jpeg",
      quality: 72,
      maxWidth: this.viewport.width,
      maxHeight: this.viewport.height,
      everyNthFrame: 1,
    });
  }

  async stopScreencast() {
    if (!this.framesDir)
      return this.frames;
    await this.send("Page.stopScreencast").catch(() => {});
    writeFileSync(join(this.framesDir, "frames.json"), JSON.stringify(this.frames, null, 2));
    return this.frames;
  }

  /** RSS and CPU of the Chrome process, sampled when a metric is read. */
  processStats() {
    try {
      const output = execFileSync("ps", ["-o", "rss=,pcpu=", "-p", String(this.process.pid)], { encoding: "utf8" }).trim();
      const [rss, cpu] = output.split(/\s+/).map(Number);
      return { rssKb: rss, cpuPercent: cpu };
    }
    catch {
      return null;
    }
  }

  async close() {
    await this.stopScreencast().catch(() => {});
    try {
      this.connection.ws.close();
    }
    catch {}
    try {
      this.process.kill("SIGTERM");
    }
    catch {}
  }
}

function keyCode(key) {
  const map = { Enter: 13, Escape: 27, Tab: 9, Backspace: 8, ArrowUp: 38, ArrowDown: 40, ArrowLeft: 37, ArrowRight: 39, Delete: 46 };
  return map[key] ?? 0;
}
