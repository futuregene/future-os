// @vitest-environment jsdom
/**
 * The poll callback's null-`start` guard (`useFutureLoginFlow.ts:104`,
 * `if (!current) return;`) — the one window in which the callback can run
 * against a `start` that a *later* render already nulled.
 *
 * The window is real and narrow: `usePolling` updates `callbackRef.current`
 * during render (`lib/usePolling.ts:35`) but clears the interval in a passive
 * effect cleanup. So between the commit of the render that nulls `start`
 * (`cancel()`, or the `authorized` arm) and React's passive-effect flush, the
 * old interval is still installed and its `tick` still has `active === true`,
 * so it calls the *new* callback — the one closed over `start === null`.
 *
 * **Why this file does not use `act()`.** Measured, not assumed: the identical
 * sequence wrapped in `act()` never opens the window, because `act()` flushes
 * the passive cleanup before it returns, so the interval is already cleared
 * before any timer can fire. Outside `act()` React commits the render and
 * schedules the cleanup as its own task — which is what a real browser does.
 * That is why the earlier `unreachable-in-this-environment` waiver for this line
 * was wrong: the fixture is not exotic, it is the everyday non-`act()` ordering,
 * and only the test harness was hiding it. So this file mounts its own root with
 * `react-dom/client` and drives the loop by hand, and fake-timers only the two
 * functions `usePolling` owns (`setInterval`/`clearInterval`) plus `Date` (the
 * back-off gate compares `Date.now()` against a reserved slot and must be held
 * open on purpose, see below). React's own scheduling stays real, which is the
 * whole point.
 */
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useFutureLoginFlow } from "./useFutureLoginFlow";

// The harness used by every other test in this module (`src/test/renderHook.ts`)
// turns this on at import time and wraps renders in `act()`; here it must be off.
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = false;

const mocks = vi.hoisted(() => ({
  pollFutureLogin: vi.fn(),
  startFutureLogin: vi.fn(),
}));

vi.mock("../../integrations/agent/providers", () => ({
  pollFutureLogin: mocks.pollFutureLogin,
  startFutureLogin: mocks.startFutureLogin,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

type Flow = ReturnType<typeof useFutureLoginFlow>;

function loginStart() {
  return {
    userCode: "ABCD",
    verificationUri: "https://example.com/device",
    verificationUriComplete: "https://example.com/device?code=ABCD",
    interval: 2,
    expiresIn: 30,
    deviceCode: "device-code",
  };
}

/**
 * One real turn of the event loop. `setImmediate` is not faked, so this lets
 * React commit, run passive effects, and drain promise continuations.
 */
function loopTurn(): Promise<void> {
  return new Promise(resolve => setImmediate(resolve));
}

async function settle(turns = 6) {
  for (let i = 0; i < turns; i += 1)
    await loopTurn();
}

interface Mounted {
  current: () => Flow;
  /** How many times a faked `clearInterval` has been called, in total. */
  cleared: () => number;
  unmount: () => void;
}

const mounted: Mounted[] = [];

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
  vi.setSystemTime(0);
  const realClearInterval = window.clearInterval.bind(window);
  // Counted through the spy itself (`Mounted.cleared`), so the test can tell
  // "the interval is still installed" from "the cleanup already ran".
  vi.spyOn(window, "clearInterval").mockImplementation(((id?: number) =>
    realClearInterval(id as never)) as typeof window.clearInterval);
  mounted.length = 0;
  mocks.pollFutureLogin.mockReset().mockResolvedValue({ status: "pending" });
  mocks.startFutureLogin.mockReset().mockResolvedValue(loginStart());
});

afterEach(() => {
  for (const m of mounted.splice(0))
    m.unmount();
  vi.restoreAllMocks();
  vi.useRealTimers();
  (globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = false;
});

function mountFlow(): Mounted {
  let latest: Flow | null = null;
  function Probe() {
    latest = useFutureLoginFlow(() => {});
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  root.render(createElement(Probe));
  const record: Mounted = {
    current: () => latest as Flow,
    cleared: () => vi.mocked(window.clearInterval).mock.calls.length,
    unmount: () => {
      root.unmount();
      container.remove();
    },
  };
  mounted.push(record);
  return record;
}

it("returns on the tick that fires with a null `start`, instead of dereferencing it", async () => {
  const flow = mountFlow();
  await settle();
  expect(flow.current().phase).toBe("idle");

  await flow.current().begin();
  await settle();
  expect(flow.current().phase).toBe("waiting");
  expect(flow.current().start).not.toBeNull();
  // `begin()`'s first tick polls immediately and reserves the next slot at
  // `Date.now() + intervalRef` — 2s from here. Move the clock past it so that
  // the guard at line 104 is the *only* thing standing between the stale tick
  // and `current.deviceCode` (the back-off gate at line 118 would otherwise
  // return first, and the test would pass with the guard deleted).
  expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);
  vi.setSystemTime(3_000);
  expect(flow.current().start).not.toBeNull();

  // The commit that nulls `start`. From here the pending passive cleanup is the
  // only thing that may still clear the interval.
  const clearedBeforeCancel = flow.cleared();
  flow.current().cancel();

  let gapOpened = false;
  for (let i = 0; i < 10 && !gapOpened; i += 1) {
    await loopTurn();
    if (flow.current().start === null && flow.cleared() === clearedBeforeCancel) {
      gapOpened = true;
      // The gap: the null-`start` render is committed, the old interval is still
      // installed (cleanup pending) and its `active` is still true, so this fires
      // `callbackRef.current` — the new callback, closed over `start === null`.
      vi.advanceTimersByTime(5_000);
    }
  }
  // Without this the rest of the test could pass for the wrong reason (no tick
  // fired at all, or the cleanup had already run so nothing was installed).
  expect(gapOpened).toBe(true);

  // Settle before asserting: anything the stale tick scheduled commits on a
  // later turn, so reading `phase` straight after `advanceTimersByTime` would
  // assert the pre-tick value and pass either way. The settle also lets the
  // pending passive cleanup run — which is the second half of the claim: the
  // interval was installed only until that flush.
  await settle();
  expect(flow.cleared()).toBeGreaterThan(clearedBeforeCancel);

  // What the guard buys, and what fails if line 104 is deleted — verified by
  // running this file against a copy of the hook with the guard removed (the
  // mechanism, the measurement and the counterfactual are recorded in
  // docs/testing/module-desktop.md §8.18): without it the callback runs past the
  // reserved-slot gate and evaluates `current.deviceCode` on null, the TypeError
  // lands in the poll's `catch`, and the dialog flips to "error" with a
  // null-dereference message. With it, the tick is a no-op and the flow stays
  // cancelled.
  expect(flow.current().phase).toBe("idle");
  expect(flow.current().message).toBeNull();
  // Not discriminating on its own (a null dereference throws before the call),
  // but it is the user-visible half: the stale tick must not poll again.
  expect(mocks.pollFutureLogin).toHaveBeenCalledTimes(1);
});
