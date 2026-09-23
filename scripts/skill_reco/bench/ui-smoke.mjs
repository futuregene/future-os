// Render the demo's page in jsdom against the running server, and fail loudly on any error the page
// logs. This is the cheap functional check for a UI that just changed shape: it catches a renamed
// field or a deleted element, which a syntax check cannot.
//
//   node server.mjs &            # on 8791
//   NODE_PATH=/Users/geilige/future-os/node_modules node ui-smoke.mjs
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { JSDOM } from "jsdom";

const here = path.dirname(fileURLToPath(import.meta.url));
const base = process.env.DEMO_URL ?? "http://127.0.0.1:8791";
const html = await (await fetch(base)).text();
const status = await (await fetch(`${base}/api/status`)).json();

const errors = [];
const dom = new JSDOM(html, { url: base, runScripts: "outside-only", pretendToBeVisual: true });
const { window } = dom;
window.fetch = (url, init) => {
  const target = String(url).startsWith("http") ? String(url) : new URL(url, base).toString();
  return fetch(target, init);
};
window.addEventListener("error", (event) => errors.push(`error: ${event.message}`));
window.console.error = (...args) => errors.push(`console.error: ${args.join(" ")}`);

// The page's own script, evaluated in the jsdom window.
const app = await readFile(path.join(here, "..", "public", "app.js"), "utf8");
window.eval(app);

// Let loadStatus() and the first render settle.
await new Promise((resolve) => setTimeout(resolve, 900));

const text = (id) => window.document.getElementById(id)?.textContent?.trim() ?? "";
const has = (id) => Boolean(window.document.getElementById(id));
const checks = [
  ["页面里不再有已删除的元素 #force", !has("force")],
  ["状态行显示 roster 数量", /141/.test(text("status"))],
  ["阈值来自服务器（没有 fits）", status.thresholds.fits === undefined && status.thresholds.noneGate === 0.15],
  ["未输入时 verdict 区已渲染", text("verdict").length > 0],
  ["页面脚本无错误", errors.length === 0],
];
console.log("=== UI 冒烟（jsdom 渲染真实页面 + 真实 /api/status）===");
for (const [label, ok] of checks) console.log(`  ${ok ? "✓" : "✗"} ${label}`);
console.log(`  status: ${text("status")}`);
console.log(`  verdict: ${text("verdict").slice(0, 60)}`);
if (errors.length) for (const e of errors.slice(0, 5)) console.log(`  ! ${e}`);

// Now the part that matters: drive a real query through the real endpoint and check the render.
const query = "帮我把这张照片转成水彩风格";
window.document.getElementById("query").value = query;
const payload = await (
  await fetch(`${base}/api/suggest`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ query, limit: 200 }),
  })
).json();
console.log("\n=== 端到端 payload 字段与 UI 的契约 ===");
const contract = [
  ["payload.verdict 存在", "verdict" in payload],
  ["payload.gate.noneProbability 存在", typeof payload.gate?.noneProbability === "number"],
  ["payload.top[] 有 name/p", payload.top?.every((t) => "name" in t && "p" in t)],
  ["payload.rank.usage/ms 存在", "usage" in (payload.rank ?? {}) && "ms" in (payload.rank ?? {})],
  ["payload 里没有已删除的 verification", payload.verification === undefined],
  ["payload 里没有已删除的 top[].verified", payload.top?.every((t) => t.verified === undefined)],
];
for (const [label, ok] of contract) console.log(`  ${ok ? "✓" : "✗"} ${label}`);

const failed = [...checks, ...contract].filter(([, ok]) => !ok).length;
console.log(`\n${failed === 0 ? "全部通过" : `${failed} 项失败`}`);
process.exit(failed === 0 ? 0 : 1);
