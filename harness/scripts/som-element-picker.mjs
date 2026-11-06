#!/usr/bin/env node
/**
 * Prototype: write a UI end-to-end case by letting a vision model pick the element.
 *
 * The model only ever chooses a NUMBER from a Set-of-Mark overlay; geometry comes
 * from the DOM and the click is a real CDP mouse event, so no model coordinate is
 * involved. Use it while authoring a case, then freeze the element it picked into a
 * `data-testid` selector and let harness/ run the case like any other.
 *
 * See docs/testing/ui-e2e-model-selection.md for measurements and pitfalls
 * (assert on *added* text, and always give the model an "id 0 = nothing fits" exit).
 *
 * Usage:
 *   FUTURE_API_KEY=... node harness/scripts/som-element-picker.mjs \
 *     --url http://localhost:5399/shot/ --objective "打开技能列表" [--chrome <path>]
 *
 * Exits 0 when the click produced new on-screen text, 1 otherwise.
 */
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const argv = process.argv.slice(2);
const arg = (name, dflt) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : dflt;
};
const URL_ = arg('url', 'http://localhost:5399/shot/');
const OBJECTIVE = arg('objective', '');
const API = arg('api', 'https://future-os.cn/api/v1/chat/completions');
const MODEL = arg('model', 'qwen3.5-flash');
const CHROME = arg('chrome', '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome');
const KEY = process.env.FUTURE_API_KEY;
const PORT = Number(arg('port', '9345'));

if (!OBJECTIVE) { console.error('missing --objective'); process.exit(2); }
if (!KEY) { console.error('missing FUTURE_API_KEY'); process.exit(2); }

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const chrome = spawn(CHROME, [
  '--headless=new', `--remote-debugging-port=${PORT}`,
  `--user-data-dir=${fs.mkdtempSync(path.join(os.tmpdir(), 'som-'))}`,
  '--hide-scrollbars', '--no-first-run', '--no-default-browser-check', 'about:blank',
], { stdio: 'ignore' });

async function pageSocket() {
  for (let i = 0; i < 80; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = list.find((t) => t.type === 'page');
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await sleep(250);
  }
  throw new Error('no page target');
}

const ws = new WebSocket(await pageSocket());
await new Promise((res) => ws.addEventListener('open', res));
let seq = 0;
const pending = new Map();
ws.addEventListener('message', (ev) => {
  const m = JSON.parse(ev.data);
  if (m.id && pending.has(m.id)) {
    const { resolve, reject } = pending.get(m.id);
    pending.delete(m.id);
    m.error ? reject(new Error(JSON.stringify(m.error))) : resolve(m.result);
  }
});
const send = (method, params = {}) => {
  const id = ++seq;
  ws.send(JSON.stringify({ id, method, params }));
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
};
const evaluate = async (expression) => {
  const r = await send('Runtime.evaluate', { expression, returnByValue: true });
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 200));
  return r.result.value;
};

await send('Page.enable');
await send('Runtime.enable');
await send('Emulation.setDeviceMetricsOverride',
  { width: 1280, height: 850, deviceScaleFactor: 1, mobile: false });
await send('Page.navigate', { url: URL_ });
await sleep(2200);

const collect = () => evaluate(`(() => {
  const out = [], seen = new Set();
  for (const el of document.querySelectorAll('button,[role=button],a,input,textarea,select')) {
    const r = el.getBoundingClientRect();
    if (r.width < 8 || r.height < 8) continue;
    if (r.bottom < 4 || r.top > innerHeight - 4 || r.right < 4 || r.left > innerWidth - 4) continue;
    const label = (el.getAttribute('aria-label') || el.innerText || el.value || '')
      .trim().replace(/\\s+/g, ' ').slice(0, 36);
    if (!label) continue;
    const k = label + '|' + Math.round(r.x) + ',' + Math.round(r.y);
    if (seen.has(k)) continue;
    seen.add(k);
    out.push({ label, x: Math.round(r.x), y: Math.round(r.y),
               w: Math.round(r.width), h: Math.round(r.height) });
  }
  return JSON.stringify(out);
})()`);

async function markedScreenshot(els) {
  await evaluate(`(() => {
    document.getElementById('__som__')?.remove();
    const box = document.createElement('div');
    box.id = '__som__';
    box.style.cssText = 'position:fixed;inset:0;pointer-events:none;z-index:2147483647';
    ${JSON.stringify(els)}.forEach((e, i) => {
      const d = document.createElement('div');
      d.style.cssText = 'position:absolute;left:' + e.x + 'px;top:' + e.y + 'px;width:' + e.w +
        'px;height:' + e.h + 'px;border:2px solid #e53935;box-sizing:border-box';
      const n = document.createElement('span');
      n.textContent = String(i + 1);
      n.style.cssText = 'position:absolute;left:0;top:-16px;background:#e53935;color:#fff;' +
        'font:bold 11px Arial;padding:1px 4px;border-radius:2px';
      d.appendChild(n); box.appendChild(d);
    });
    document.body.appendChild(box);
  })()`);
  const shot = await send('Page.captureScreenshot', { format: 'png' });
  await evaluate(`document.getElementById('__som__')?.remove()`);
  return shot.data;
}

const els = JSON.parse(await collect());
const before = await evaluate('document.body.innerText');
const png = await markedScreenshot(els);

const res = await fetch(API, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${KEY}` },
  body: JSON.stringify({
    model: MODEL,
    messages: [
      { role: 'system', content:
        '你在一张桌面应用截图上做端到端测试。每个可交互元素都画了红框并标了编号。\n' +
        '只输出 JSON：{"id": 编号, "reasoning": "一句话"}。id 必须是图上的数字；\n' +
        '如果图上没有能完成目标的元素，输出 id 0，不要挑一个“最像”的凑数。' },
      { role: 'user', content: [
        { type: 'text', text: `目标：${OBJECTIVE}\n可选元素：\n` +
          els.map((e, i) => `${i + 1}. ${e.label}`).join('\n') },
        { type: 'image_url', image_url: { url: `data:image/png;base64,${png}` } }] }],
    max_tokens: 300, temperature: 0.2,
  }),
});
const txt = (await res.json()).choices?.[0]?.message?.content || '';
const picked = Number((txt.match(/"id"\s*:\s*(\d+)/) || [])[1] ?? 0);
const target = picked > 0 ? els[picked - 1] : null;

if (!target) {
  console.log(JSON.stringify({ objective: OBJECTIVE, refused: true, raw: txt.trim().slice(0, 120) }));
  ws.close(); chrome.kill();
  process.exit(0);
}

for (const type of ['mousePressed', 'mouseReleased']) {
  await send('Input.dispatchMouseEvent',
    { type, x: target.x + target.w / 2, y: target.y + target.h / 2, button: 'left', clickCount: 1 });
  await sleep(40);
}
await sleep(700);

// only text that was NOT on screen before counts as evidence of a new surface
const after = await evaluate('document.body.innerText');
const seen = new Set(before.split('\n'));
const added = after.split('\n').filter((l) => l.trim() && !seen.has(l));

console.log(JSON.stringify({ objective: OBJECTIVE, picked, label: target.label,
  added: added.slice(0, 6), pass: added.length > 0 }, null, 1));
ws.close();
chrome.kill();
process.exit(added.length > 0 ? 0 : 1);
