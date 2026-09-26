// @vitest-environment jsdom
import type { ThreadRunInfo } from "./useThreadStore";
import { beforeEach, describe, expect, it } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useUnreadThreads } from "./useUnreadThreads";

const STORAGE_KEY = "future.unreadThreads";

function info(status: ThreadRunInfo["status"]): ThreadRunInfo {
  return { status, endedAt: null };
}

function mount(runInfo: Record<string, ThreadRunInfo | undefined>, activeThreadId: string | null) {
  const ref = { activeThreadId, runInfo };
  const harness = renderHook(() => useUnreadThreads(ref.runInfo, ref.activeThreadId));
  return { harness, ref };
}

describe("useUnreadThreads", () => {
  beforeEach(() => {
    sessionStorage.clear();
  });

  it("marks a thread when its run is observed finishing", () => {
    const { harness, ref } = mount({ t1: info("running") }, null);
    expect([...harness.current]).toEqual([]);

    ref.runInfo = { t1: info("completed") };
    harness.rerender();
    expect([...harness.current]).toEqual(["t1"]);
    expect(sessionStorage.getItem(STORAGE_KEY)).toBe("[\"t1\"]");
    harness.unmount();
  });

  it("counts queued and waiting_approval as in progress, and failed as a finish", () => {
    const { harness, ref } = mount({ a: info("queued"), b: info("waiting_approval") }, null);

    // Neither queued → running nor waiting_approval → running is an edge.
    ref.runInfo = { a: info("running"), b: info("running") };
    harness.rerender();
    expect([...harness.current]).toEqual([]);

    ref.runInfo = { a: info("failed"), b: info("running") };
    harness.rerender();
    expect([...harness.current]).toEqual(["a"]);
    harness.unmount();
  });

  it("ignores a thread first seen already finished, and cancelled", () => {
    const { harness, ref } = mount({ t1: info("completed"), t2: info("running") }, null);
    expect([...harness.current]).toEqual([]);

    ref.runInfo = { t1: info("completed"), t2: info("cancelled") };
    harness.rerender();
    // cancelled is terminal but not "finished" for the unread dot.
    expect([...harness.current]).toEqual([]);
    harness.unmount();
  });

  it("clears the mark when the thread is opened", () => {
    const { harness, ref } = mount({ t1: info("running") }, null);
    ref.runInfo = { t1: info("completed") };
    harness.rerender();
    expect([...harness.current]).toEqual(["t1"]);

    ref.activeThreadId = "t1";
    harness.rerender();
    expect([...harness.current]).toEqual([]);
    expect(sessionStorage.getItem(STORAGE_KEY)).toBe("[]");
    harness.unmount();
  });

  it("clears the mark when the thread is switched away from", () => {
    const { harness, ref } = mount({ t1: info("running"), t2: info("running") }, "t1");
    ref.runInfo = { t1: info("completed"), t2: info("completed") };
    harness.rerender();
    expect([...harness.current]).toEqual(["t1", "t2"]);

    ref.activeThreadId = "t2";
    harness.rerender();
    // Both edges clear: t1 was left, t2 was entered.
    expect([...harness.current]).toEqual([]);
    harness.unmount();
  });

  it("keeps the mark while the thread stays open (no edge, no clearing)", () => {
    const { harness, ref } = mount({ t1: info("running") }, "t1");
    ref.runInfo = { t1: info("completed") };
    harness.rerender();
    expect([...harness.current]).toEqual(["t1"]);

    // Staying put — even an unrelated re-render — must not clear the dot.
    harness.rerender();
    expect([...harness.current]).toEqual(["t1"]);
    harness.unmount();
  });

  it("restores the session's marks on mount and ignores a corrupt payload", () => {
    sessionStorage.setItem(STORAGE_KEY, "[\"t7\"]");
    const restored = mount({}, null);
    expect([...restored.harness.current]).toEqual(["t7"]);
    restored.harness.unmount();

    sessionStorage.setItem(STORAGE_KEY, "{not json");
    const corrupt = mount({}, null);
    expect([...corrupt.harness.current]).toEqual([]);
    corrupt.harness.unmount();
  });

  it("forgets the status of threads that left the store, so a new run re-marks", () => {
    const { harness, ref } = mount({ t1: info("running") }, null);
    ref.runInfo = { t1: info("completed") };
    harness.rerender();
    expect([...harness.current]).toEqual(["t1"]);

    // The thread disappears (unpinned / filtered out) — its status entry is dropped.
    ref.runInfo = {};
    harness.rerender();

    ref.runInfo = { t1: info("running") };
    harness.rerender();
    ref.runInfo = { t1: info("completed") };
    harness.rerender();
    expect([...harness.current]).toEqual(["t1"]);
    harness.unmount();
  });

  it("skips undefined entries in the run-info map", () => {
    const { harness, ref } = mount({ t1: info("running"), t2: undefined }, null);
    ref.runInfo = { t1: info("completed"), t2: undefined };
    harness.rerender();
    expect([...harness.current]).toEqual(["t1"]);
    harness.unmount();
  });

  it("survives a sessionStorage that is unavailable", () => {
    // Private mode: every touch of `sessionStorage` throws, so both the initial
    // read and the persist effect must be guarded.
    const original = Object.getOwnPropertyDescriptor(window, "sessionStorage");
    Object.defineProperty(window, "sessionStorage", {
      configurable: true,
      get() {
        throw new Error("SecurityError: storage disabled");
      },
    });
    try {
      const { harness, ref } = mount({ t1: info("running") }, null);
      expect([...harness.current]).toEqual([]);

      ref.runInfo = { t1: info("completed") };
      // Without the write guard the persist effect would throw during commit.
      expect(() => harness.rerender()).not.toThrow();
      // The in-memory set still applies for this session.
      expect([...harness.current]).toEqual(["t1"]);
      harness.unmount();
    }
    finally {
      if (original)
        Object.defineProperty(window, "sessionStorage", original);
      else
        Reflect.deleteProperty(window, "sessionStorage");
    }
  });
});
