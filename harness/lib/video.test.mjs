/**
 * The clip's clock, which the analysis leans on.
 *
 * Every screenshot the judge sees carries "录像 12.4s", and the filmstrip is
 * sampled in video time — so the two agree only if the recorded clip's clock is
 * the one the labels use. It is not the wall clock: a single frame is capped at
 * 5 s, and a burst of frames is floored at 10 ms each. These tests pin both ends
 * of that rule, because getting it wrong is invisible: the labels still look
 * plausible and point at the wrong frame.
 *
 * The 40 ms floor this replaced stretched a 2.9 s case into a 4.7 s file — a 60%
 * slow-down during exactly the animations a reviewer cares about.
 *
 * Run with `node --test harness/lib/video.test.mjs`.
 */
import test from "node:test";
import assert from "node:assert/strict";
import { frameDuration, videoClock } from "./video.mjs";

test("a burst of frames is floored at 10ms, not 40ms", () => {
  assert.equal(frameDuration({ ts: 0 }, { ts: 3 }), 0.01);
});

test("an idle gap is capped at 5s instead of freezing the picture for a minute", () => {
  assert.equal(frameDuration({ ts: 0 }, { ts: 60_000 }), 5);
});

test("the last frame has no next: it gets the default 120ms", () => {
  assert.equal(frameDuration({ ts: 0 }, undefined), 0.12);
});

test("the clock is the sum of those durations, sampled against frame timestamps", () => {
  const frames = [
    { file: "a", ts: 1_000 },
    { file: "b", ts: 1_500 },
    { file: "c", ts: 21_500 },
  ];
  const clock = videoClock(frames);
  // 0.5 s for the second frame, 5 s (capped) for the third, 0.12 s tail.
  assert.equal(clock.durationMs, 5_620);
  assert.deepEqual(clock.samples, [[1_000, 0], [1_500, 0.5], [21_500, 5.5]]);
  assert.equal(clock.startMs, 1_000);
  assert.equal(clock.endMs, 21_500);
});

test("a long run is sampled, not stored frame by frame", () => {
  const frames = Array.from({ length: 4_000 }, (_, index) => ({ file: `f${index}`, ts: index * 20 }));
  const clock = videoClock(frames);
  assert.ok(clock.samples.length <= 401, `${clock.samples.length} samples`);
  assert.equal(clock.samples.at(-1)[0], frames.at(-1).ts);
  // 4000 frames × 20 ms = 80 s; the sample grid must reach the end of it.
  assert.ok(clock.durationMs >= 79_900 && clock.durationMs <= 80_200, `${clock.durationMs}`);
});

test("no frames means no clock, so a caller can fall back instead of dividing by zero", () => {
  assert.equal(videoClock([]), null);
});
