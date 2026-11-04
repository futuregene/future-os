/**
 * Proof that the performance thresholds can fail.
 *
 * A threshold that has never been seen to fail is indistinguishable from a
 * comment. These tests take the numbers the harness actually measured and check
 * both directions: real noise passes, a regression of the size we claim to
 * catch fails. Run with `node --test harness/lib/perf.test.mjs`.
 */
import test from "node:test";
import assert from "node:assert/strict";
import { resolveLimit, drift, slope, percentile } from "./perf.mjs";

const boot = { median: 2065, min: 1969, max: 2168, n: 6, noise: 1.05 };

test("observed noise passes with the tolerance the case uses", () => {
  // The worst boot this machine produced was 2168ms; the case allows baseline+25%.
  const { limit } = resolveLimit({ metric: "bootMs", tolerance: 0.25, max: 4000 }, boot);
  assert.ok(2168 <= limit, `${2168} should pass under ${limit}`);
});

test("a 30% regression fails", () => {
  const { limit } = resolveLimit({ metric: "bootMs", tolerance: 0.25, max: 4000 }, boot);
  const regressed = boot.median * 1.3; // 2684ms — a plausible "someone added a query" boot
  assert.ok(regressed > limit, `${regressed} should fail over ${limit}`);
});

test("the ceiling still catches a catastrophe even with a generous baseline", () => {
  const { limit } = resolveLimit({ metric: "bootMs", tolerance: 2, max: 4000 }, boot);
  assert.equal(limit, 4000, "ceiling wins when the tolerance alone would allow more");
});

test("a missing baseline degrades to the ceiling instead of failing", () => {
  const { limit, baseline } = resolveLimit({ metric: "bootMs", max: 4000 }, null);
  assert.equal(limit, 4000);
  assert.equal(baseline, null);
});

test("signed growth metrics use an additive bound, because a ratio of a negative says nothing", () => {
  const { limit } = resolveLimit({ metric: "heapGrowth", toleranceAbs: 40, max: 100 }, { median: -6 });
  assert.equal(limit, 34);
});

test("a floor keeps tiny metrics from a meaningless multiplicative bound", () => {
  // p95 of a 5ms command: +150% is 13.5ms, which noise alone crosses.
  const { limit } = resolveLimit({ metric: "ipcP95", tolerance: 1.5, floor: 25, max: 60 }, { median: 5.4 });
  assert.equal(limit, 25);
});

test("drift separates a flat series from a leaking one", () => {
  const flat = [300, 295, 310, 300, 290, 305, 300, 298, 302, 300, 295, 305];
  assert.ok(drift(flat) < 1.1, `flat series should not drift: ${drift(flat)}`);
  const leaking = [260, 270, 285, 300, 315, 330, 350, 370, 395, 420, 450, 480];
  assert.ok(drift(leaking) > 1.5, `leaking series should drift: ${drift(leaking)}`);
});

test("slope is zero for a flat series and positive per step for a leaking one", () => {
  assert.ok(Math.abs(slope([300, 300, 300, 300])) < 1e-9);
  const perStep = slope([100, 110, 120, 130]);
  assert.ok(Math.abs(perStep - 10) < 1e-6, `expected 10ms/step, got ${perStep}`);
});

test("percentile is nearest-rank, not interpolated", () => {
  assert.equal(percentile([1, 2, 3, 4, 5], 95), 5);
  assert.equal(percentile([], 95), null);
});
