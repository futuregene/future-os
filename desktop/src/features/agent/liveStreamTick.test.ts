import { describe, expect, it } from "vitest";
import { createLiveTick } from "./liveStreamTick";

/** A virtual clock so the coalescing/rate-limit rules are deterministic. */
function virtualClock() {
  let current = 0;
  const sleeps: number[] = [];
  return {
    now: () => current,
    sleep: async (ms: number) => {
      sleeps.push(ms);
      current += ms;
    },
    advance: (ms: number) => {
      current += ms;
    },
    sleeps,
  };
}

/** Let the tick's fire-and-forget `drain()` settle. */
const settle = () => new Promise<void>(resolve => setTimeout(resolve, 0));

/** A manually-settled promise, so a projection can be held in flight. */
function deferred() {
  let resolve: () => void = () => {};
  const promise = new Promise<void>((r) => {
    resolve = r;
  });
  return { promise, resolve: () => resolve() };
}

describe("createLiveTick", () => {
  it("runs a leading projection immediately and coalesces a burst into one trailing run", async () => {
    const clock = virtualClock();
    const runs: number[] = [];
    const tick = createLiveTick({
      isActive: () => true,
      project: async () => {
        runs.push(clock.now());
      },
      now: clock.now,
      sleep: clock.sleep,
      intervalMs: 100,
    });

    // Leading call: no interval has elapsed, so it must not wait.
    tick.request();
    await settle();
    expect(runs).toEqual([0]);
    expect(clock.sleeps).toEqual([]);

    // A burst of pushes during the following interval collapses to ONE run.
    for (let i = 0; i < 6; i++)
      tick.request();
    await settle();
    expect(runs).toEqual([0, 100]);
    expect(clock.sleeps).toEqual([100]);
  });

  it("spaces successive projections by the interval", async () => {
    const clock = virtualClock();
    const runs: number[] = [];
    const tick = createLiveTick({
      isActive: () => true,
      project: async () => {
        runs.push(clock.now());
      },
      now: clock.now,
      sleep: clock.sleep,
      intervalMs: 100,
    });

    tick.request();
    await settle();
    // Three pushed ticks, each arriving right after the previous run.
    for (let round = 0; round < 3; round++) {
      tick.request();
      await settle();
    }
    expect(runs).toEqual([0, 100, 200, 300]);
  });

  it("never overlaps projections", async () => {
    const clock = virtualClock();
    let inFlight = 0;
    let maxInFlight = 0;
    let gate = deferred();
    const tick = createLiveTick({
      isActive: () => true,
      project: async () => {
        inFlight += 1;
        maxInFlight = Math.max(maxInFlight, inFlight);
        await gate.promise;
        inFlight -= 1;
      },
      now: clock.now,
      sleep: clock.sleep,
      intervalMs: 0,
    });

    tick.request();
    await settle();
    tick.request();
    tick.request();
    await settle();
    expect(maxInFlight).toBe(1);

    gate.resolve();
    await settle();
    gate = deferred();
    gate.resolve();
    await settle();
    expect(maxInFlight).toBe(1);
  });

  it("stops permanently and drops a queued request", async () => {
    const clock = virtualClock();
    const runs: number[] = [];
    const tick = createLiveTick({
      isActive: () => true,
      project: async () => {
        runs.push(runs.length);
      },
      now: clock.now,
      sleep: clock.sleep,
      intervalMs: 100,
    });

    tick.request();
    await settle();
    expect(runs).toHaveLength(1);

    tick.request();
    tick.stop();
    await settle();
    expect(runs).toHaveLength(1);

    // A request after stop() must never resurrect the tick — the settle path
    // owns the final render and a late intermediate write would clobber it.
    tick.request();
    await settle();
    expect(runs).toHaveLength(1);
  });

  it("stops driving when the owner goes inactive", async () => {
    const clock = virtualClock();
    let active = true;
    const runs: number[] = [];
    const tick = createLiveTick({
      isActive: () => active,
      project: async () => {
        runs.push(runs.length);
      },
      now: clock.now,
      sleep: clock.sleep,
      intervalMs: 100,
    });

    tick.request();
    await settle();
    expect(runs).toHaveLength(1);

    active = false;
    tick.request();
    await settle();
    expect(runs).toHaveLength(1);
  });

  it("runs afterProject once per projection", async () => {
    const clock = virtualClock();
    let after = 0;
    const tick = createLiveTick({
      isActive: () => true,
      project: async () => {},
      afterProject: () => {
        after += 1;
      },
      now: clock.now,
      sleep: clock.sleep,
      intervalMs: 100,
    });

    tick.request();
    await settle();
    tick.request();
    await settle();
    expect(after).toBe(2);
  });

  it("coalesces pushes that arrive while a projection is in flight", async () => {
    const clock = virtualClock();
    const runs: number[] = [];
    const gate = deferred();
    let firstRun = true;
    const tick = createLiveTick({
      isActive: () => true,
      project: async () => {
        runs.push(runs.length);
        if (firstRun) {
          firstRun = false;
          await gate.promise;
        }
      },
      now: clock.now,
      sleep: clock.sleep,
      intervalMs: 0,
    });

    tick.request();
    await settle();
    // Five pushes land while the first projection is still awaiting.
    for (let i = 0; i < 5; i++)
      tick.request();
    gate.resolve();
    await settle();
    // One trailing run for the whole burst — not five.
    expect(runs).toHaveLength(2);
  });
});
