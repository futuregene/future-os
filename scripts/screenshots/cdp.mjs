#!/usr/bin/env node
/**
 * Minimal Chrome DevTools Protocol driver for screenshot capture.
 *
 * Drives an already-running Chrome/Edge (headless or visible) over CDP: set a
 * viewport, tap/type/scroll like a real user, and save PNGs. Mobile captures
 * rely on device-metrics emulation, which Chrome resets when the CDP session
 * ends — so one scenario must run inside a single invocation of this script.
 *
 * Usage:
 *   node scripts/screenshots/cdp.mjs --port 9222 --match localhost:5299 \
 *        --w 390 --h 844 --url http://localhost:5299/shot/ --steps steps.json
 *
 * Options:
 *   --port N        CDP port of the browser (default 9222)
 *   --match TEXT    pick the page target whose URL contains TEXT
 *   --w/--h N       viewport in CSS pixels
 *   --scale N       device scale factor (2 = retina-sized PNGs)
 *   --mobile BOOL   mobile emulation on/off (default true)
 *   --touch BOOL    dispatch touch sequences for taps (default true)
 *   --url URL       navigate before running the steps
 *   --settle MS     grace period after navigating (default 500)
 *   --ready JS      expression that turns truthy once the app has rendered;
 *                   polled every 150ms (much faster than a fixed settle)
 *   --ready-timeout MS  give up waiting for --ready (default 30000)
 *   --steps FILE    JSON array of steps (see below)
 *   --frames DIR    record a screencast into DIR as frame-NNNNN.jpg plus
 *                   timing.json (frame timestamps, for pacing on encode).
 *                   capture.py turns that into an mp4; see docs.
 *   --fps N         screencast frame-rate ceiling (default 12)
 *
 * Steps:
 *   {"eval": js}                 evaluate in the page (result printed)
 *   {"wait": ms}                 pause
 *   {"shot": path}               capture a PNG
 *   {"tap": "aria-label"}        real input tap on that element
 *   {"tapText": "visible text"}  real input tap by first line of text
 *   {"hover": "aria-label"}      move the mouse over that element
 *   {"type": "text"}             insert text into the focused element
 *   {"key": "Escape"}            press one key
 *   {"scroll": [dx, dy]}         wheel scroll
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import WebSocket from "ws";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const index = args.indexOf(`--${name}`);
  return index >= 0 ? args[index + 1] : fallback;
};

const PORT = Number(opt("port", "9222"));
const MATCH = opt("match", "");
const WIDTH = Number(opt("w", "1280"));
const HEIGHT = Number(opt("h", "860"));
const SCALE = Number(opt("scale", "2"));
const MOBILE = opt("mobile", "true") !== "false";
const TOUCH = opt("touch", "true") !== "false";
const URL_ARG = opt("url", "");
const STEPS_FILE = opt("steps", "");
const SETTLE = Number(opt("settle", "500"));
const READY = opt("ready", "");
const READY_TIMEOUT = Number(opt("ready-timeout", "30000"));
const FRAMES_DIR = opt("frames", "");
const FPS = Number(opt("fps", "12"));
/** `--annotate false` disables the pointer indicator and mark callouts. */
const ANNOTATE = opt("annotate", "true") !== "false";

const steps = STEPS_FILE ? JSON.parse(readFileSync(STEPS_FILE, "utf8")) : [];

const targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const page = targets.find(target => target.type === "page" && target.url.includes(MATCH))
  ?? targets.find(target => target.type === "page");
if (!page) {
  console.error(`cdp: no page target on port ${PORT}`);
  process.exit(1);
}

const socket = new WebSocket(page.webSocketDebuggerUrl, { maxPayload: 512 * 1024 * 1024 });
await new Promise(resolve => socket.on("open", resolve));

let nextId = 1;
const pending = new Map();
socket.on("message", (raw) => {
  const message = JSON.parse(raw.toString());
  if (message.id && pending.has(message.id)) {
    pending.get(message.id)(message);
    pending.delete(message.id);
  }
  if (message.method === "Page.screencastFrame")
    onScreencastFrame(message.params);
});

function send(method, params = {}) {
  const id = nextId++;
  socket.send(JSON.stringify({ id, method, params }));
  return new Promise(resolve => pending.set(id, resolve));
}

/**
 * On-screen pointer / touch indicator and annotations.
 *
 * Injected into the page so they appear in both screenshots and video. A video
 * of a phone UI is hard to follow without one: nothing shows where the finger
 * landed. The overlay is a fixed, pointer-events-none div, so it never
 * interferes with what the app does.
 *
 * `mark` steps add persistent callouts (a numbered badge plus a label) which are
 * what make a still screenshot self-explanatory.
 */
const OVERLAY_ID = "__shot_overlay";

async function ensureOverlay() {
  if (!ANNOTATE)
    return false;
  await evaluate(`
(() => {
  if (document.getElementById(${JSON.stringify(OVERLAY_ID)})) return true;
  const layer = document.createElement('div');
  layer.id = ${JSON.stringify(OVERLAY_ID)};
  layer.style.cssText = [
    'position:fixed', 'inset:0', 'z-index:2147483647',
    'pointer-events:none', 'font-family:-apple-system,system-ui,sans-serif',
  ].join(';');
  layer.innerHTML = '<div data-role="pointer" style="position:absolute;opacity:0"></div>'
    + '<div data-role="marks" style="position:absolute;inset:0"></div>';
  document.body.appendChild(layer);
  return true;
})()`);
}

/** Show the pointer, optionally with a tap ring that fades. */
async function showPointer(x, y, tapped) {
  if (!ANNOTATE)
    return false;
  await evaluate(`
(() => {
  const layer = document.getElementById(${JSON.stringify(OVERLAY_ID)});
  if (!layer) return false;
  const dot = layer.querySelector('[data-role=pointer]');
  dot.style.cssText = [
    'position:absolute', 'left:${x}px', 'top:${y}px', 'width:14px', 'height:14px',
    'margin:-7px 0 0 -7px', 'border-radius:9999px', 'opacity:1',
    'background:rgba(43,108,255,.85)', 'border:2px solid #fff',
    'box-shadow:0 1px 6px rgba(16,20,24,.45)', 'transition:opacity .15s ease',
  ].join(';');
  if (${tapped ? "true" : "false"}) {
    // A short-lived ring is what reads as "a tap happened here".
    const ring = document.createElement('div');
    ring.style.cssText = [
      'position:absolute', 'left:${x}px', 'top:${y}px', 'width:14px', 'height:14px',
      'margin:-7px 0 0 -7px', 'border-radius:9999px',
      'border:3px solid rgba(43,108,255,.9)',
      'animation:__shot_ring .5s ease-out forwards',
    ].join(';');
    if (!document.getElementById('__shot_ring_style')) {
      const style = document.createElement('style');
      style.id = '__shot_ring_style';
      style.textContent = '@keyframes __shot_ring{from{transform:scale(1);opacity:.9}to{transform:scale(3.2);opacity:0}}';
      document.head.appendChild(style);
    }
    layer.appendChild(ring);
    setTimeout(() => ring.remove(), 600);
  }
  return true;
})()`);
}

/** Add a numbered callout near (x, y), with an optional label.
 *
 * The badge sits exactly on the point (that is its meaning); the label is
 * offset to the side so it never covers what it is pointing at. `dx`/`dy` and
 * `side` let a scenario move a callout when two targets are close together.
 */
async function addMark(x, y, label, index, options = {}) {
  if (!ANNOTATE)
    return false;
  const dx = options.dx ?? 0;
  const dy = options.dy ?? 0;
  // The anchor is zero-sized, so placing the label to the *right* means pinning
  // its left edge (`left:30px`); pinning `right:30px` would push it off the
  // opposite side of the anchor.
  const placement = options.side === "left" ? "right:30px" : "left:30px";
  await evaluate(`
(() => {
  const layer = document.getElementById(${JSON.stringify(OVERLAY_ID)});
  if (!layer) return false;
  const marks = layer.querySelector('[data-role=marks]');
  const mark = document.createElement('div');
  mark.style.cssText = 'position:absolute;left:${x + dx}px;top:${y + dy}px';
  const badge = '<span style="position:absolute;left:-11px;top:-11px;display:flex;align-items:center;'
    + 'justify-content:center;min-width:22px;height:22px;padding:0 5px;border-radius:9999px;'
    + 'background:#2b6cff;color:#fff;font-size:13px;font-weight:600;white-space:nowrap;'
    + 'box-shadow:0 1px 6px rgba(16,20,24,.4)">' + ${JSON.stringify(String(index))} + '</span>';
  const labelText = ${JSON.stringify(label)};
  const label = labelText
    ? '<span style="position:absolute;top:-11px;${placement};max-width:280px;padding:3px 8px;'
      + 'border-radius:6px;background:rgba(16,20,24,.88);color:#fff;font-size:13px;line-height:1.45;'
      + 'white-space:nowrap;overflow:hidden;text-overflow:ellipsis;'
      + 'box-shadow:0 1px 6px rgba(16,20,24,.3)">' + labelText + '</span>'
    : '';
  mark.innerHTML = badge + label;
  marks.appendChild(mark);
  return true;
})()`);
}

async function clearMarks() {
  await evaluate(`
(() => {
  const layer = document.getElementById(${JSON.stringify(OVERLAY_ID)});
  const marks = layer && layer.querySelector('[data-role=marks]');
  if (marks) marks.innerHTML = '';
  return true;
})()`);
}

async function hidePointer() {
  await evaluate(`
(() => {
  const layer = document.getElementById(${JSON.stringify(OVERLAY_ID)});
  const dot = layer && layer.querySelector('[data-role=pointer]');
  if (dot) dot.style.opacity = '0';
  return true;
})()`);
}

/**
 * Screencast recording.
 *
 * Chrome streams JPEG frames and waits for an ack per frame, so the ack is what
 * paces capture: while a step (`wait`) is in progress, one frame is written and
 * acked. Frames land in `--frames DIR` together with `timing.json`, and
 * `capture.py` encodes them with the real inter-frame delays so the video keeps
 * the scenario's pace instead of a fixed frame rate.
 */
const frames = [];
let frameIndex = 0;
let screencastDone = Promise.resolve();

function onScreencastFrame(params) {
  const file = join(FRAMES_DIR, `frame-${String(frameIndex++).padStart(5, "0")}.jpg`);
  writeFileSync(file, Buffer.from(params.data, "base64"));
  frames.push({ file, at: Date.now() - screencastStart });
  void send("Page.screencastFrameAck", { sessionId: params.sessionId });
}

let screencastStart = 0;

async function startScreencast() {
  mkdirSync(FRAMES_DIR, { recursive: true });
  screencastStart = Date.now();
  await send("Page.startScreencast", {
    format: "jpeg",
    quality: 80,
    maxWidth: WIDTH,
    maxHeight: HEIGHT,
    everyNthFrame: Math.max(1, Math.round(30 / FPS)),
  });
}

async function stopScreencast() {
  await send("Page.stopScreencast");
  writeFileSync(join(FRAMES_DIR, "timing.json"), JSON.stringify({
    fps: FPS,
    width: WIDTH,
    height: HEIGHT,
    scale: SCALE,
    durationMs: Date.now() - screencastStart,
    frames,
  }, null, 2));
  console.log(`cdp: recorded ${frames.length} frames into ${FRAMES_DIR}`);
}

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

/** Steps that could not find their target: the scenario did not do what it says. */
let failures = 0;
/** Marks are numbered across the whole scenario. */
let markCounter = 0;

async function evaluate(expression) {
  const response = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (response.result?.exceptionDetails)
    console.error("cdp: eval error:", JSON.stringify(response.result.exceptionDetails).slice(0, 400));
  return response.result?.result?.value;
}

/**
 * Locate an element by its accessible name: `aria-label` when present, else the
 * element's own text for interactive roles. Matching the accessibility tree (what
 * a screen reader and the browser's own tooling see) keeps the scenarios honest
 * about what a user can actually reach.
 */
const locate = (query, byText) => `
(() => {
  const wanted = ${JSON.stringify(query)};
  const INTERACTIVE = 'button,[role=button],[role=menuitem],[role=tab],[role=switch],[role=checkbox],a[href],input,textarea';
  let target = null;
  if (${byText}) {
    const matches = [...document.querySelectorAll('div,span,button,p,a')]
      .filter(el => ((el.innerText || '').trim().split('\\n')[0] || '').startsWith(wanted));
    target = matches[matches.length - 1] || null;
  } else {
    const labelled = [...document.querySelectorAll('[aria-label]')];
    const interactive = [...document.querySelectorAll(INTERACTIVE)];
    target = labelled.find(el => el.getAttribute('aria-label') === wanted)
      || labelled.find(el => (el.getAttribute('aria-label') || '').includes(wanted))
      || interactive.find(el => (el.innerText || '').trim().split('\\n')[0] === wanted)
      || interactive.find(el => ((el.innerText || '').trim().split('\\n')[0] || '').startsWith(wanted))
      || null;
  }
  if (!target) return null;
  target.scrollIntoView({ block: 'center' });
  const rect = target.getBoundingClientRect();
  return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2,
           w: rect.width, h: rect.height,
           label: target.getAttribute('aria-label') || (target.innerText || '').slice(0, 30) };
})()`;

async function mouse(type, x, y, extra = {}) {
  await send("Input.dispatchMouseEvent", {
    type,
    x,
    y,
    button: "left",
    clickCount: type === "mouseMoved" ? 0 : 1,
    buttons: type === "mousePressed" ? 1 : 0,
    ...extra,
  });
}

async function interact(query, byText, action) {
  const spot = await evaluate(locate(query, byText));
  if (!spot) {
    console.error(`cdp: target not found: ${query}`);
    failures += 1;
    return null;
  }
  if (spot.w === 0 || spot.h === 0) {
    // Clicking a zero-sized box would land at the page corner and do nothing.
    // Controls revealed on hover (e.g. a sidebar row's "..." button) need an
    // explicit {"hover": ...} step on their container first.
    console.error(`cdp: target is not visible (${Math.round(spot.w)}x${Math.round(spot.h)}): ${query}`);
    failures += 1;
    return null;
  }
  await mouse("mouseMoved", spot.x, spot.y);
  // Keep the overlay under the pointer for screenshots and video.
  await showPointer(spot.x, spot.y, action === "tap");
  if (action === "tap") {
    await sleep(80);
    if (TOUCH) {
      // Inside a react-native-web ScrollView the scroll responder wins over a
      // synthetic mouse click, so a touch sequence is what the component
      // listens for. Sending both a touch and a mouse sequence would toggle
      // controls twice.
      await send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: spot.x, y: spot.y, id: 1 }] });
      await sleep(70);
      await send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    }
    else {
      await mouse("mousePressed", spot.x, spot.y);
      await sleep(60);
      await mouse("mouseReleased", spot.x, spot.y);
    }
    await sleep(150);
  }
  console.log(`cdp: ${action} ${spot.label} @${Math.round(spot.x)},${Math.round(spot.y)} ${Math.round(spot.w)}x${Math.round(spot.h)}`);
  return spot;
}

await send("Page.enable");
await send("Runtime.enable");
await send("Emulation.setDeviceMetricsOverride", {
  width: WIDTH,
  height: HEIGHT,
  deviceScaleFactor: SCALE,
  mobile: MOBILE,
});
if (TOUCH)
  await send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });

// Start recording before navigating so the video includes the first paint.
if (FRAMES_DIR)
  await startScreencast();

if (URL_ARG) {
  await send("Page.navigate", { url: URL_ARG });
  if (READY) {
    // Poll for the app's own "first paint" signal instead of guessing a fixed
    // duration: a warm bundle is ready in ~1s, a cold one can take 20s.
    const started = Date.now();
    for (;;) {
      if (await evaluate(READY) === true)
        break;
      if (Date.now() - started > READY_TIMEOUT) {
        console.error(`cdp: app not ready after ${READY_TIMEOUT} ms: ${READY}`);
        failures += 1;
        break;
      }
      await sleep(150);
    }
    console.log(`cdp: ready in ${Date.now() - started} ms`);
  }
  await sleep(SETTLE);
}
if (ANNOTATE)
  await ensureOverlay();

for (const step of steps) {
  if (step.eval !== undefined) {
    const value = await evaluate(step.eval);
    if (step.log !== false && value !== undefined)
      console.log("cdp: eval:", JSON.stringify(value).slice(0, 1200));
  }
  if (step.wait !== undefined)
    await sleep(step.wait);
  if (step.hover !== undefined)
    await interact(step.hover, false, "hover");
  if (step.tap !== undefined)
    await interact(step.tap, false, "tap");
  if (step.tapText !== undefined)
    await interact(step.tapText, true, "tap");
  if (step.pointer !== undefined) {
    // "pointer": [x, y] — show the indicator at a raw position (e.g. to point at
    // something that is not an interactive element).
    await showPointer(step.pointer[0], step.pointer[1], step.pointerTap === true);
  }
  if (step.pointerHide === true)
    await hidePointer();
  if (step.marksClear === true)
    await clearMarks();
  if (step.marks !== undefined) {
    // "marks": [{ "at": "aria-label" | [x, y], "label": "…" }, …] — numbered
    // callouts, numbered in the order given (continuing an existing set).
    for (const mark of step.marks) {
      let x = mark.x;
      let y = mark.y;
      if (mark.at !== undefined) {
        const spot = await evaluate(locate(mark.at, false));
        if (!spot) {
          console.error(`cdp: mark target not found: ${mark.at}`);
          failures += 1;
          continue;
        }
        x = spot.x;
        y = spot.y;
      }
      if (x === undefined || y === undefined) {
        console.error("cdp: mark needs either \"at\" or \"x\"/\"y\"");
        failures += 1;
        continue;
      }
      markCounter += 1;
      await addMark(x, y, mark.label ?? "", markCounter, { dx: mark.dx, dy: mark.dy, side: mark.side });
      console.log(`cdp: mark ${markCounter} ${mark.label ?? ""}`.trim());
    }
  }
  if (step.type !== undefined)
    await send("Input.insertText", { text: step.type });
  if (step.key !== undefined) {
    await send("Input.dispatchKeyEvent", { type: "keyDown", key: step.key, text: step.key.length === 1 ? step.key : undefined });
    await send("Input.dispatchKeyEvent", { type: "keyUp", key: step.key });
  }
  if (step.scroll !== undefined) {
    const [deltaX, deltaY] = step.scroll;
    await send("Input.dispatchMouseEvent", {
      type: "mouseWheel",
      x: WIDTH / 2,
      y: HEIGHT / 2,
      deltaX,
      deltaY,
      button: "none",
    });
  }
  if (step.shot !== undefined) {
    await sleep(250);
    const capture = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
    writeFileSync(step.shot, Buffer.from(capture.result.data, "base64"));
    console.log("cdp: shot", step.shot);
  }
}

if (FRAMES_DIR)
  await stopScreencast();
socket.close();
if (failures > 0) {
  console.error(`cdp: ${failures} step(s) could not reach their target`);
  process.exitCode = 1;
}
