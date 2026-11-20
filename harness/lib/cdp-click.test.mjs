/**
 * Proof that a click carries no driver latency.
 *
 * `perf-thread-switch`'s 294ms baseline was ~180ms driver time: `click` slept
 * 60ms between press and release and 120ms after it, so every latency measured
 * through it was really "the click action finished". These tests pin the
 * property back down without a browser: the press and release go out back to
 * back, and what follows the release is a condition, not a timer.
 *
 * Run with `node --test harness/lib/cdp-click.test.mjs` (also in
 * `make test-ui-harness`).
 */
import test from "node:test";
import assert from "node:assert/strict";

import { CdpPage } from "./cdp.mjs";

/** A page whose transport and evaluate are recorded instead of sent. */
function fakePage({ touch = false } = {}) {
  const sent = [];
  const page = Object.create(CdpPage.prototype);
  page.touch = touch;
  page.locate = async () => ({ x: 10, y: 10, w: 40, h: 20 });
  page.send = async (method, params) => {
    sent.push({ method, params, at: performance.now() });
    return {};
  };
  page.evaluate = async (expression) => {
    sent.push({ method: "evaluate", expression, at: performance.now() });
    return "quiet";
  };
  return { page, sent };
}

const indexOf = (sent, method, type) => sent.findIndex(entry => entry.method === method && entry.params?.type === type);

test("press and release are dispatched back to back", async () => {
  const { page, sent } = fakePage();
  await page.click({ text: "会话" });
  const pressed = indexOf(sent, "Input.dispatchMouseEvent", "mousePressed");
  const released = indexOf(sent, "Input.dispatchMouseEvent", "mouseReleased");
  assert.ok(pressed >= 0 && released > pressed, "both events are dispatched");
  assert.equal(released, pressed + 1, "nothing is sent between press and release");
  assert.ok(sent[released].at - sent[pressed].at < 20, `press→release took ${Math.round(sent[released].at - sent[pressed].at)}ms`);
});

test("touch clicks have the same property", async () => {
  const { page, sent } = fakePage({ touch: true });
  await page.click({ text: "会话" });
  const start = indexOf(sent, "Input.dispatchTouchEvent", "touchStart");
  const end = indexOf(sent, "Input.dispatchTouchEvent", "touchEnd");
  assert.ok(start >= 0 && end > start, "both touch events are dispatched");
  const between = sent.slice(start + 1, end);
  assert.equal(between.length, 1, "one frame of press duration, nothing else");
  assert.match(between[0].expression, /requestAnimationFrame/, "the press duration is a frame, not a timer");
  assert.ok(sent[end].at - sent[start].at < 40, "touchStart→touchEnd carries no fixed sleep");
});

test("a settled click waits on a condition, not a timer", async () => {
  const { page, sent } = fakePage();
  await page.click({ text: "会话" });
  const afterRelease = sent.slice(indexOf(sent, "Input.dispatchMouseEvent", "mouseReleased") + 1);
  const settle = afterRelease.find(entry => entry.method === "evaluate");
  assert.ok(settle, "the settle runs in the page");
  assert.match(settle.expression, /MutationObserver/, "the settle observes DOM changes");
  assert.match(settle.expression, /requestAnimationFrame/, "the settle waits for painted frames");
});

test("settle:false leaves the waiting to the caller and costs no time", async () => {
  const { page, sent } = fakePage();
  const started = performance.now();
  await page.click({ text: "会话" }, { settle: false });
  const elapsed = performance.now() - started;
  assert.equal(sent.filter(entry => entry.method === "evaluate").length, 0, "no in-page settle");
  assert.ok(elapsed < 20, `click({ settle: false }) took ${Math.round(elapsed)}ms — a sleep came back`);
});

test("an unreachable target still fails loudly", async () => {
  const { page } = fakePage();
  page.locate = async () => null;
  await assert.rejects(() => page.click({ text: "不存在" }), /target not found/);
});
