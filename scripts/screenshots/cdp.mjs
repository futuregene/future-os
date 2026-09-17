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
import { readFileSync, writeFileSync } from "node:fs";
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
});

function send(method, params = {}) {
  const id = nextId++;
  socket.send(JSON.stringify({ id, method, params }));
  return new Promise(resolve => pending.set(id, resolve));
}

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

/** Steps that could not find their target: the scenario did not do what it says. */
let failures = 0;

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

socket.close();
if (failures > 0) {
  console.error(`cdp: ${failures} step(s) could not reach their target`);
  process.exitCode = 1;
}
