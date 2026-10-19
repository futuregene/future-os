import { HistoryPageHiddenError, commitHistoryPage } from "../historyPaging";
import { emptyTimeline } from "@future-os/remote-client/timeline";
import type { SyncEngine } from "@future-os/remote-client/syncEngine";

type Timeline = ReturnType<typeof emptyTimeline>;

/**
 * Committing an already-downloaded older page into the lane.
 *
 * The page is a download in hand, so the only question here is whether it can
 * *land*: a lane that is off screen, or one whose window has moved on since the
 * page was read, must refuse the commit instead of publishing rows into a
 * conversation that is no longer there. `commitHistoryPage` answers both by
 * failing the commit — this suite drives those refusals against a hand-built
 * lane, which is the half of the contract the hook-level tests reach only one
 * call at a time.
 */
describe("commitHistoryPage", () => {
  /**
   * The smallest stand-in for a lane: it records what was asked and runs the
   * mutator when the test says the lane's turn has come.
   */
  function fakeEngine(canCommit: boolean) {
    let queued: ((live: Timeline) => Timeline) | null = null;
    const engine = {
      canCommit: jest.fn(() => canCommit),
      subscribe: jest.fn(() => () => undefined),
      mutate: jest.fn((_sessionId: string, apply: (live: Timeline) => Timeline) => {
        queued = apply;
      }),
    };
    return {
      engine: engine as unknown as SyncEngine,
      mutate: engine.mutate,
      /** The lane's turn: applies whatever the commit queued into it. */
      run: (live: Timeline): Timeline => {
        if (!queued) throw new Error("nothing queued into the lane");
        const apply = queued;
        queued = null;
        return apply(live);
      },
    };
  }

  test("a window that moved on before the commit is a cancellation, not a page", async () => {
    const lane = fakeEngine(true);

    // The caller's own window check fails first: the page was read against a
    // cursor the conversation has since replaced.
    await expect(
      commitHistoryPage(lane.engine, "s1", emptyTimeline(), () => false, new AbortController().signal),
    ).rejects.toThrow("history_page_cancelled");
    // Nothing was queued into the lane — a stale window must not even be offered
    // to the engine, which would publish it once the lane came back.
    expect(lane.mutate).not.toHaveBeenCalled();
  });

  test("an app that is already backgrounded refuses the commit outright", async () => {
    const lane = fakeEngine(false);

    // The lane cannot publish while the app is off screen, so the caller is told
    // immediately rather than waiting out the page timeout for an answer that is
    // already known.
    await expect(
      commitHistoryPage(lane.engine, "s1", emptyTimeline(), () => true, new AbortController().signal),
    ).rejects.toBeInstanceOf(HistoryPageHiddenError);
    expect(lane.mutate).not.toHaveBeenCalled();
  });

  test("a lane that went dark while the commit waited leaves the timeline untouched", async () => {
    const lane = fakeEngine(true);
    let current = true;
    const live = emptyTimeline();

    const commit = commitHistoryPage(
      lane.engine, "s1", emptyTimeline(), () => current, new AbortController().signal,
    );
    // The page queued behind the lane's own work; its turn arrives after the
    // window is gone.
    current = false;
    const rejection = expect(commit).rejects.toThrow("history_page_cancelled");
    // The mutator itself refuses: it returns the timeline unchanged, so a late
    // turn cannot prepend a page whose window no longer exists.
    expect(lane.run(live)).toBe(live);
    await rejection;
  });
});
