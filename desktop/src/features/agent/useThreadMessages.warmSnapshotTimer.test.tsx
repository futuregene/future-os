// @vitest-environment jsdom
/**
 * The loading-timer callback's warm-snapshot guard —
 * `useThreadMessages.ts:375`, `if (hasWarmSnapshotRef.current) return;` (tag
 * `D54`, falsified by the independent review in
 * `docs/testing/review-desktop.md` §19.1/§27, covered here).
 *
 * The waived reason claimed an *absolute*: "the commit that warms the ref always
 * flips `loadingThread` to false, the indicator effect re-runs (`deps =
 * [loadingThread]`) and *clears this timer* before it can fire". The claim is
 * about a **commit**, but `reloadThreadHistory` writes the ref synchronously
 * inside the load's own continuation (line 252) and merely *queues* the two
 * state updates that make `loadingThread` false (line 256); the cleanup that
 * would clear the timer lives in the **passive phase** of that later commit (the
 * effect keyed on `loadingThread`, line 401). So between the two the timer is
 * still installed *and* the ref is already warm, and a timer firing there takes
 * the return at 375.
 *
 * **Why the load here resolves outside `act()`.** Measured, not assumed (see
 * §8.20 of `docs/testing/module-desktop.md`): `act()` flushes the passive
 * cleanup of the queued commit before it returns, so the timer is gone before it
 * can fire — the *harness*, not the machine, closes the window. The app itself
 * renders outside `act()`, so this file mounts its own root, fake-timers only
 * `setTimeout`/`clearTimeout`/`Date` (React's own scheduling stays real) and
 * drains the load's microtask chain by hand. This is the construction
 * `useFutureLoginFlow.staleTick.test.ts` used for `D41`.
 *
 * **What the assertion is.** `setLoadingIndicator(true)` has exactly **one**
 * call site — inside this timer, past the guard — so the observable difference
 * the guard makes is that the indicator must NOT come on for a snapshot that is
 * already warm when the timer fires. Without the return at 375 the timer reaches
 * `setLoadingIndicator(true)`, and the effect's `indicatorShownAtRef` bookkeeping
 * then holds the indicator on for its 200 ms minimum. That is why the assertion
 * below is on the indicator (and on the committed state still reporting
 * "loading"), not merely on a line having been executed: a `DA:1` read alone
 * would not distinguish "returned early" from "the timer never fired".
 */
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { loadThreadHistory } from "./history/loadThreadHistory";
import { clearThreadMessageSnapshots } from "./threadMessageCache";
import { useThreadMessages } from "./useThreadMessages";

const { page, latest, runs, failed } = vi.hoisted(() => ({
  page: vi.fn(),
  latest: vi.fn(),
  runs: vi.fn(),
  failed: vi.fn((messages: unknown) => messages),
}));

vi.mock("../../integrations/storage/threadStore", () => ({
  getSessionEntriesPage: page,
  getLatestRun: latest,
  listRuns: runs,
  getRun: vi.fn(),
}));

// `recoverFailedRuns` is **synchronous** in the real module
// (`history/runHistoryProjection`) and must stay synchronous here: mocked
// `async`, `loadThreadHistory` returns a Promise where it needs an array,
// `finalMessages.map` is not a function, and the read lands in its own `catch`
// as `{ status: "failed" }` — the hook then never reaches line 252 and a `DA:0`
// read would say nothing about the guard. The direct-call test below is the
// three-line diagnostic that catches exactly that.
vi.mock("./threadRunProjection", () => ({
  applyJournalRunOutcomes: (messages: unknown) => messages,
  applyRunMetadata: (messages: unknown) => messages,
  recoverAbortedTurns: async (messages: unknown) => messages,
  recoverFailedRuns: failed,
  buildStreamingPreview: vi.fn(),
  mergeStreamingPreview: (messages: unknown) => messages,
}));

// The module's usual harness (`src/test/renderHook.ts`) turns this on at import
// time and wraps renders in `act()`; the non-`act()` ordering is the point here,
// so it must be off.
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = false;

const DELAY_MS = 200;

function history(ids: string[], nextOffset: number, hasMore = false) {
  return {
    entries: ids.map(id => ({
      id,
      kind: "user",
      role: "user",
      createdAtMs: Date.parse("2026-01-01T00:00:00Z"),
      blocks: [{ kind: "text", text: id }],
    })),
    nextOffset,
    hasMore,
  };
}

let current: ReturnType<typeof useThreadMessages>;

function Harness() {
  current = useThreadMessages({ threadId: "synthetic", agentSessionId: "synthetic-session" });
  return null;
}

/** One real turn of the event loop: React's commit and its passive effects. */
function loopTurn(): Promise<void> {
  return new Promise(resolve => setImmediate(resolve));
}

async function settle(turns = 4) {
  for (let i = 0; i < turns; i += 1)
    await loopTurn();
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
  clearThreadMessageSnapshots();
  latest.mockResolvedValue(null);
  runs.mockResolvedValue([]);
});

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
  (globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = false;
});

/**
 * Mount with a cold load in flight, so the indicator effect arms its showTimer:
 * `initialSnapshot === null` → `blockingLoad === true` → `loadingThread === true`
 * with `hasWarmSnapshotRef` still false.
 */
async function mountWithPendingLoad() {
  let resolvePage: ((value: unknown) => void) | undefined;
  page.mockImplementation(() => new Promise((resolve) => {
    resolvePage = resolve;
  }));
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  root.render(createElement(Harness));
  await settle();
  expect(page).toHaveBeenCalledTimes(1);
  expect(current.loadingThread).toBe(true);
  expect(current.loadingIndicator).toBe(false);
  return {
    resolve: () => resolvePage?.(history(["u1"], 0, false)),
    unmount: () => {
      root.unmount();
      container.remove();
    },
  };
}

it("diagnostic: this harness's read path reaches `loaded`, not loadThreadHistory's `catch`", async () => {
  // Three lines that make the coverage read below meaningful: if the mocked
  // `recoverFailedRuns` (or any other projection the load awaits) is ever made
  // asynchronous, the read silently becomes `{ status: "failed" }` and the hook
  // never reaches line 252 at all.
  page.mockResolvedValueOnce(history(["u1"], 0, false));
  const result = await loadThreadHistory("synthetic");
  expect(result.status).toBe("loaded");
  expect(result.status === "loaded" ? result.messages.map(m => m.content) : []).toEqual(["u1"]);
});

it("control: with the snapshot still cold, the timer's own branch shows the indicator", async () => {
  const mounted = await mountWithPendingLoad();

  vi.advanceTimersByTime(DELAY_MS - 1);
  await settle(1);
  expect(current.loadingIndicator).toBe(false);

  vi.advanceTimersByTime(1);
  await settle(2);
  // The contrast the next test needs: the same timer, the same delay, and the
  // indicator comes on — because `hasWarmSnapshotRef.current` was false.
  expect(current.loadingIndicator).toBe(true);

  mounted.unmount();
});

it("covers line 375: the timer fires inside the load's own window, with the ref already warm", async () => {
  const mounted = await mountWithPendingLoad();

  vi.advanceTimersByTime(DELAY_MS - 1);
  await settle(1);
  expect(current.loadingIndicator).toBe(false);

  // The load lands. Line 252 writes `hasWarmSnapshotRef` now; line 256 only
  // *queues* `setBlockingLoad(false)`, whose commit (and passive cleanup) is a
  // macrotask that no microtask drain can reach.
  mounted.resolve();
  for (let i = 0; i < 30; i += 1)
    await Promise.resolve();

  // Witness that the read ran to the end of `loadThreadHistory` before the last
  // millisecond is allowed to pass — otherwise this test could "pass" on a load
  // that never completed, i.e. by the timer firing into a cold ref and the
  // indicator assertion below failing for a different reason.
  expect(failed).toHaveBeenCalledTimes(1);

  // The last millisecond of the delay, before that commit's cleanup can run.
  vi.advanceTimersByTime(1);

  // The last committed render still reports the loading state — i.e. the
  // callback read a warm ref while the state the effect keys on was unchanged.
  // This is the window the waived reason denied.
  expect(current.loadingThread).toBe(true);
  // The assertion the guard buys (single caller of `setLoadingIndicator(true)`):
  // a warm snapshot arriving must not flash the indicator. Delete the return at
  // 375 and the timer falls through to `setLoadingIndicator(true)`.
  expect(current.loadingIndicator).toBe(false);

  await settle();
  expect(current.loadingThread).toBe(false);
  expect(current.loadingIndicator).toBe(false);
  expect(current.messages.map(message => message.content)).toEqual(["u1"]);

  mounted.unmount();
});
