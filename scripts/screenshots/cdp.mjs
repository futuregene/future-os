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
 *   --inject FILE   run this JS file once the app is ready, before the steps
 *                   (variant styling: the same screen with a different icon or
 *                   colour choice)
 *   --measure FILE  write the geometry collected by `offsets` steps
 *   --baseline FILE compare against an earlier `--measure` run (shows movement)
 *   --pointer BOOL  draw the pointer indicator on taps/hovers
 *                   (default: on while recording, off for stills)
 *   --annotate BOOL remove every overlay when false (default true)
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
 *   {"pointer": [x, y]}          show the pointer indicator at that point
 *   {"pointerHide": true}        hide the pointer indicator
 *   {"marks": [{at|selector|x,y,label,dx,dy,side}]}  numbered callouts
 *   {"marksClear": true}         clear the callouts
 *   {"offsets": {targets,axis,mode}}  box + pixel-offset diagram (see docs)
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import WebSocket from "ws";

/** scripts/screenshots/ — relative `inject` paths resolve against it. */
const SCRIPTS_DIR = dirname(fileURLToPath(import.meta.url));

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
/** `--annotate false` removes every overlay (pointer, callouts, diagrams). */
const ANNOTATE = opt("annotate", "true") !== "false";
/** `--measure FILE` writes the geometry collected by `offsets` steps. */
const MEASURE_FILE = opt("measure", "");
/** `--baseline FILE` compares against an earlier `--measure` run. */
const BASELINE_FILE = opt("baseline", "");
/** `--inject FILE` runs a JS file after the app is ready (variant styling). */
const INJECT_FILE = opt("inject", "");
/**
 * `--pointer BOOL` draws the pointer indicator that follows taps/hovers.
 *
 * Default: on while recording a screencast (footage of a phone is unfollowable
 * without it), off for stills — a product shot should not gain a stray dot just
 * because the scenario had to tap something. A `{"pointer": …}` step or
 * `--pointer true` turns it on explicitly for a still.
 */
const SHOW_POINTER = ANNOTATE && opt("pointer", FRAMES_DIR ? "true" : "false") !== "false";
const baseline = BASELINE_FILE
  ? JSON.parse(readFileSync(BASELINE_FILE, "utf8")).elements ?? []
  : null;

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

/** Steps that could not find their target: the scenario did not do what it says. */
let failures = 0;
/** Marks are numbered across the whole scenario. */
let markCounter = 0;

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
  if (!SHOW_POINTER)
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
 * Layout geometry measurement and offset diagrams.
 *
 * `offsets` measures the elements it is given and draws their boxes plus the
 * pixel offset between them (or from the viewport edge) as a dimension line with
 * the number on it. With `--baseline` (a JSON written by an earlier run with
 * `--measure`) the same diagram shows *displacement*: how far each element moved
 * versus that baseline, which is how you compare two builds or two versions.
 *
 * Measurements are in CSS pixels at the emulated viewport size — the numbers in
 * the diagram are the layout's own units, not device pixels, so they match what
 * a designer quotes.
 */
const measurements = [];

/** Geometry of one target, in viewport CSS pixels. */
const MEASURE_JS = (target) => `
(() => {
  ${MATCH_FN}
  const spec = ${JSON.stringify(target)};
  let element = null;
  if (spec.selector) {
    element = document.querySelector(spec.selector);
  } else {
    element = __shotMatch(spec.at);
  }
  if (!element) return null;
  const r = element.getBoundingClientRect();
  return {
    name: spec.name || spec.at || spec.selector,
    matched: element.getAttribute('aria-label') || (element.innerText || '').trim().split('\\n')[0],
    x: r.left, y: r.top, width: r.width, height: r.height,
    right: r.right, bottom: r.bottom,
  };
})()`;

async function measureTargets(targets) {
  const results = [];
  for (const target of targets) {
    const geometry = await evaluate(MEASURE_JS(target));
    if (!geometry) {
      console.error(`cdp: measure target not found: ${target.at ?? target.selector}`);
      failures += 1;
      continue;
    }
    const name = target.name ?? geometry.name;
    // Log the match: a name that resolved to the wrong element is otherwise
    // invisible until someone reads the diagram.
    console.log(`cdp: measured ${name} -> "${geometry.matched}" at ${Math.round(geometry.x)},${Math.round(geometry.y)} ${Math.round(geometry.width)}×${Math.round(geometry.height)}`);
    results.push({ ...geometry, name });
  }
  return results;
}

/** Draw one element's box, with its name and size. */
async function drawBox(box, tone) {
  await evaluate(`
(() => {
  const layer = document.getElementById(${JSON.stringify(OVERLAY_ID)});
  if (!layer) return false;
  const marks = layer.querySelector('[data-role=marks]');
  const el = document.createElement('div');
  el.style.cssText = 'position:absolute;left:${box.x}px;top:${box.y}px;'
    + 'width:${box.width}px;height:${box.height}px;'
    + 'outline:1px solid ${tone};outline-offset:-1px;background:${tone}11';
  const label = document.createElement('span');
  label.style.cssText = 'position:absolute;left:0;top:-18px;padding:1px 5px;border-radius:4px;'
    + 'background:${tone};color:#fff;font-size:11px;font-weight:600;white-space:nowrap';
  label.textContent = ${JSON.stringify(box.name)} + ' ' + Math.round(${box.width}) + '×' + Math.round(${box.height});
  el.appendChild(label);
  marks.appendChild(el);
  return true;
})()`);
}

/**
 * Draw a dimension line between two points with the distance on it.
 * `axis` is "y" (vertical distance) or "x" (horizontal distance).
 */
async function drawDimension(axis, from, to, text, tone) {
  await evaluate(`
(() => {
  const layer = document.getElementById(${JSON.stringify(OVERLAY_ID)});
  if (!layer) return false;
  const marks = layer.querySelector('[data-role=marks]');
  const el = document.createElement('div');
  if (${JSON.stringify(axis)} === 'y') {
    el.style.cssText = 'position:absolute;left:${from.x}px;top:${Math.min(from.y, to.y)}px;'
      + 'width:0;height:${Math.abs(to.y - from.y)}px;border-left:1px dashed ${tone}';
  } else {
    el.style.cssText = 'position:absolute;left:${Math.min(from.x, to.x)}px;top:${from.y}px;'
      + 'width:${Math.abs(to.x - from.x)}px;height:0;border-top:1px dashed ${tone}';
  }
  const cap = document.createElement('span');
  cap.style.cssText = 'position:absolute;white-space:nowrap;padding:1px 5px;border-radius:4px;'
    + 'background:rgba(16,20,24,.9);color:#fff;font-size:12px;font-weight:600;'
    + (${JSON.stringify(axis)} === 'y'
      ? 'left:6px;top:50%;transform:translateY(-50%)'
      : 'left:50%;top:-9px;transform:translateX(-50%)');
  cap.textContent = ${JSON.stringify(text)};
  el.appendChild(cap);
  marks.appendChild(el);
  return true;
})()`);
}

const round = value => Math.round(value * 10) / 10;

/**
 * Draw the offset diagram for a group of elements.
 *
 * mode "gap"  — consecutive gaps along `axis`, in the order given (top→bottom or
 *               left→right); this is the spacing question ("how far is this row
 *               from that one").
 * mode "edge" — each element's distance from the viewport edge on `axis`
 *               (left/top), plus its size; this is the alignment question.
 *
 * With `baseline` (same scenario captured earlier through `--measure`), each
 * element's movement since then is drawn instead: the offset *change*, in px.
 */
async function drawOffsets(spec, baseline) {
  if (!ANNOTATE)
    return [];
  const axis = spec.axis === "x" ? "x" : "y";
  const mode = spec.mode === "edge" ? "edge" : "gap";
  const tone = spec.color ?? "#e2685f";

  const boxes = await measureTargets(spec.targets ?? []);
  for (const box of boxes) {
    measurements.push({
      name: box.name, x: round(box.x), y: round(box.y),
      width: round(box.width), height: round(box.height),
    });
    await drawBox(box, tone);
  }

  if (mode === "edge") {
    const edge = axis === "y" ? 0 : 0;
    for (const box of boxes) {
      const at = axis === "y" ? box.y : box.x;
      const across = axis === "y" ? box.x + 8 : box.y;
      await drawDimension(axis === "y" ? "y" : "x",
        axis === "y" ? { x: across, y: edge } : { x: edge, y: across },
        axis === "y" ? { x: across, y: at } : { x: at, y: across },
        `${axis === "y" ? "top" : "left"} ${round(at)}px`, tone);
    }
    return boxes;
  }

  // Gaps run between consecutive elements, ordered along the axis so the labels
  // read in the same direction as the layout.
  const ordered = [...boxes].sort((a, b) => (axis === "y" ? a.y - b.y : a.x - b.x));
  for (let index = 0; index + 1 < ordered.length; index += 1) {
    const current = ordered[index];
    const next = ordered[index + 1];
    const gap = axis === "y"
      ? next.y - current.bottom
      : next.x - current.right;
    const from = axis === "y"
      ? { x: current.x + 6, y: current.bottom }
      : { x: current.right, y: current.y + 6 };
    const to = axis === "y"
      ? { x: current.x + 6, y: next.y }
      : { x: next.x, y: current.y + 6 };
    await drawDimension(axis, from, to, `${round(gap)}px`, tone);
  }

  if (baseline) {
    for (const box of boxes) {
      const before = baseline.find(entry => entry.name === box.name);
      if (!before)
        continue;
      const movedX = round(box.x - before.x);
      const movedY = round(box.y - before.y);
      const widthChange = round(box.width - before.width);
      const parts = [];
      if (movedX !== 0 || movedY !== 0)
        parts.push(`位移 ${movedX >= 0 ? "+" : ""}${movedX}, ${movedY >= 0 ? "+" : ""}${movedY}px`);
      if (widthChange !== 0)
        parts.push(`宽 ${widthChange >= 0 ? "+" : ""}${widthChange}px`);
      if (parts.length === 0)
        parts.push("无位移");
      await evaluate(`
(() => {
  const layer = document.getElementById(${JSON.stringify(OVERLAY_ID)});
  const marks = layer && layer.querySelector('[data-role=marks]');
  if (!marks) return false;
  const el = document.createElement('span');
  el.style.cssText = 'position:absolute;left:${box.x + box.width + 6}px;top:${box.y}px;'
    + 'padding:2px 6px;border-radius:4px;background:#1f6f3f;color:#fff;font-size:11px;'
    + 'font-weight:600;white-space:nowrap';
  el.textContent = ${JSON.stringify(parts.join(" · "))};
  marks.appendChild(el);
  return true;
})()`);
    }
  }
  return boxes;
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
  // Match the still captures' resolution: frames come out at the viewport size
  // unless the cap is given in device pixels, and a 1× video next to 2× stills
  // looks visibly soft (text especially).
  await send("Page.startScreencast", {
    format: "jpeg",
    quality: 92,
    maxWidth: WIDTH * SCALE,
    maxHeight: HEIGHT * SCALE,
    everyNthFrame: Math.max(1, Math.round(30 / FPS)),
  });
}

async function stopScreencast() {
  await send("Page.stopScreencast");
  writeFileSync(join(FRAMES_DIR, "timing.json"), JSON.stringify({
    fps: FPS,
    width: WIDTH,
    height: HEIGHT,
    // Frames arrive at CSS size; the encoder upscales by this factor so the mp4
    // matches the still captures' pixel dimensions.
    scale: SCALE,
    durationMs: Date.now() - screencastStart,
    frames,
  }, null, 2));
  console.log(`cdp: recorded ${frames.length} frames into ${FRAMES_DIR}`);
}

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

/** Apply a variant-styling JS file to the page. */
async function applyInjection(file) {
  const source = readFileSync(file, "utf8");
  const ok = await evaluate(`(() => { ${source}\n return true; })()`);
  console.log(`cdp: injected ${file}${ok === true ? "" : " (no return value)"}`);
  return ok === true;
}

/** Resolve an inject path: absolute, or relative to scripts/screenshots/. */
function resolveInject(value) {
  if (value.startsWith("/") || value.includes(":")) {
    if (!existsSync(value)) {
      console.error(`cdp: inject file not found: ${value}`);
      failures += 1;
      return null;
    }
    return value;
  }
  const candidate = join(SCRIPTS_DIR, value);
  if (!existsSync(candidate)) {
    console.error(`cdp: inject file not found: ${candidate}`);
    failures += 1;
    return null;
  }
  return candidate;
}

async function evaluate(expression, { required = false } = {}) {
  const response = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  const failure = response.result?.exceptionDetails;
  if (failure) {
    console.error("cdp: eval error:", JSON.stringify(failure).slice(0, 400));
    // An eval inside a scenario is an assertion about what the screen shows:
    // reporting success for it would let a broken screen be captured as if the
    // state had been proven. Internal probes (readiness, optional reads) stay
    // non-fatal.
    if (required) failures += 1;
  }
  return response.result?.result?.value;
}

/**
 * Locate an element by its accessible name: `aria-label` when present, else the
 * element's own text for interactive roles. Matching the accessibility tree (what
 * a screen reader and the browser's own tooling see) keeps the scenarios honest
 * about what a user can actually reach.
 */
/**
 * Element matching, shared by every step that names a target (tap, marks,
 * offsets).
 *
 * Ranked rather than first-hit: an element whose *own text* is exactly the
 * wanted string beats one whose aria-label merely contains it. Without the
 * ranking, `技能` matched the composer's `选择技能` button instead of the rail row
 * — a silent wrong measurement in an offset diagram, and a click on the wrong
 * control elsewhere. Lower score wins; ties keep DOM order.
 *
 *   0  aria-label is exactly the wanted name
 *   1  an interactive element's own text is exactly the wanted name
 *   2  an interactive element's own text starts with the wanted name
 *   3  aria-label contains the wanted name
 */
const MATCH_FN = `
function __shotMatch(wanted, root) {
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

const locate = (query, byText) => `
(() => {
  ${MATCH_FN}
  const wanted = ${JSON.stringify(query)};
  let target = null;
  if (${byText}) {
    const matches = [...document.querySelectorAll('div,span,button,p,a')]
      .filter(el => ((el.innerText || '').trim().split('\\n')[0] || '').startsWith(wanted));
    target = matches[matches.length - 1] || null;
  } else {
    target = __shotMatch(wanted);
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

// Variant styling: a JS file applied once the app is up, before the steps run.
// Used to render the same screen with different icon/color choices side by side
// (see `variants` in scripts/screenshots/scenarios.json). A scenario can also
// inject mid-run with an `inject` step, which is what to use when the app
// re-renders the element afterwards (React restores its own DOM).
if (INJECT_FILE)
  await applyInjection(INJECT_FILE);

for (const step of steps) {
  if (step.eval !== undefined) {
    const value = await evaluate(step.eval, { required: true });
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
  if (step.inject !== undefined) {
    // {"inject": "injects/foo.js"} — apply variant styling at this point. Use a
    // step rather than --inject when the app re-renders the element later
    // (React restores its own DOM, dropping an earlier mutation).
    const file = resolveInject(step.inject);
    if (file)
      await applyInjection(file);
  }
  if (step.marksClear === true)
    await clearMarks();
  if (step.offsets !== undefined) {
    // "offsets": { "targets": [{ "at" | "selector", "name" }], "axis", "mode" }
    // — measure the elements and draw their boxes plus the pixel offset between
    // them. With --baseline, the same boxes also show how far each moved.
    if (step.offsetsClear === true)
      await clearMarks();
    await drawOffsets(step.offsets, baseline);
  }
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
if (MEASURE_FILE) {
  writeFileSync(MEASURE_FILE, JSON.stringify({
    viewport: { width: WIDTH, height: HEIGHT, scale: SCALE },
    elements: measurements,
  }, null, 2));
  console.log(`cdp: measured ${measurements.length} element(s) into ${MEASURE_FILE}`);
}
socket.close();
if (failures > 0) {
  console.error(`cdp: ${failures} step(s) could not reach their target`);
  process.exitCode = 1;
}
