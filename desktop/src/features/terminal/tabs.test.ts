// @vitest-environment jsdom
import type { TerminalInfo } from "./types";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearTabsState,
  EMPTY_TABS,
  loadTabsState,
  migrateTabsState,
  nextTitleNumber,
  saveTabsState,
  selectTabAfterClose,
  tabsStorageKey,
} from "./tabs";
import { defaultTitle, reconcileTabs } from "./useTerminalTabs";

function info(overrides: Partial<TerminalInfo> = {}): TerminalInfo {
  return {
    id: "term_1",
    threadId: "thread-1",
    title: "Terminal 1",
    command: "/bin/bash",
    args: ["-l", "-i"],
    cwd: "/home/user/project",
    status: "running",
    exitCode: null,
    pid: 42,
    cols: 80,
    rows: 24,
    ...overrides,
  };
}

beforeEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

describe("tab state persistence", () => {
  it("round-trips the view state", () => {
    saveTabsState("thread-1", {
      active: "term_2",
      all: [
        { id: "term_1", title: "Terminal 1", titleNumber: 1, buffer: "hello", cursor: 5, cols: 80, rows: 24 },
        { id: "term_2", title: "Terminal 2", titleNumber: 2 },
      ],
    });
    const loaded = loadTabsState("thread-1");
    expect(loaded.active).toBe("term_2");
    expect(loaded.all).toHaveLength(2);
    expect(loaded.all[0]).toMatchObject({ id: "term_1", buffer: "hello", cursor: 5 });
  });

  it("drops invalid entries instead of breaking the panel", () => {
    const state = migrateTabsState({
      active: "gone",
      all: [
        { id: "term_1", title: "ok", titleNumber: 1 },
        { title: "no id" },
        { id: "term_1", title: "duplicate", titleNumber: 3 },
        null,
        { id: "term_2", titleNumber: -4 },
      ],
    });
    expect(state.all.map(tab => tab.id)).toEqual(["term_1", "term_2"]);
    expect(state.all[1]?.titleNumber).toBe(0);
    // The stale active id falls back to the first usable tab.
    expect(state.active).toBe("term_1");
  });

  it("survives unparseable and non-object stored values", () => {
    localStorage.setItem(tabsStorageKey("thread-1"), "{not json");
    expect(loadTabsState("thread-1")).toEqual(EMPTY_TABS);
    localStorage.setItem(tabsStorageKey("thread-1"), "\"a string\"");
    expect(loadTabsState("thread-1")).toEqual(EMPTY_TABS);
  });

  it("removes the entry when the last tab closes", () => {
    saveTabsState("thread-1", { active: "term_1", all: [{ id: "term_1", title: "T", titleNumber: 1 }] });
    expect(localStorage.getItem(tabsStorageKey("thread-1"))).not.toBeNull();
    saveTabsState("thread-1", { all: [] });
    expect(localStorage.getItem(tabsStorageKey("thread-1"))).toBeNull();
    clearTabsState("thread-1");
  });

  it("does not throw when storage is unavailable", () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota exceeded");
    });
    expect(() => saveTabsState("thread-1", { all: [{ id: "term_1", title: "T", titleNumber: 1 }] })).not.toThrow();
    setItem.mockRestore();
  });

  it("picks the lowest unused label number", () => {
    expect(nextTitleNumber([])).toBe(1);
    expect(nextTitleNumber([{ id: "a", title: "", titleNumber: 1 }])).toBe(2);
    expect(nextTitleNumber([
      { id: "a", title: "", titleNumber: 1 },
      { id: "c", title: "", titleNumber: 3 },
    ])).toBe(2);
  });

  it("selects the previous tab when the active one closes", () => {
    const tabs = [
      { id: "a", title: "", titleNumber: 1 },
      { id: "b", title: "", titleNumber: 2 },
      { id: "c", title: "", titleNumber: 3 },
    ];
    expect(selectTabAfterClose(tabs, "b")).toBe("a");
    expect(selectTabAfterClose(tabs, "a")).toBe("b");
    expect(selectTabAfterClose(tabs, "missing")).toBe("a");
  });
});

describe("reconciling stored tabs with the server", () => {
  it("keeps a known running session and its screen", () => {
    const stored = {
      active: "term_1",
      all: [{ id: "term_1", title: "Terminal 1", titleNumber: 1, buffer: "old", cursor: 3 }],
    };
    const merged = reconcileTabs(stored, [info()]);
    expect(merged.all).toHaveLength(1);
    expect(merged.all[0]).toMatchObject({ buffer: "old", cursor: 3, missing: false });
    expect(merged.all[0]?.exitCode).toBeUndefined();
  });

  it("picks up the exit code of a session the server still retains", () => {
    const stored = { active: "term_1", all: [{ id: "term_1", title: "Terminal 1", titleNumber: 1 }] };
    const merged = reconcileTabs(stored, [info({ status: "exited", exitCode: 130, pid: null })]);
    expect(merged.all[0]?.exitCode).toBe(130);
  });

  it("marks a tab the server no longer has instead of dropping it", () => {
    const stored = { active: "term_1", all: [{ id: "term_1", title: "Terminal 1", titleNumber: 1, buffer: "final" }] };
    const merged = reconcileTabs(stored, []);
    expect(merged.all).toHaveLength(1);
    expect(merged.all[0]?.missing).toBe(true);
    // The final screen survives, so the user can still read it.
    expect(merged.all[0]?.buffer).toBe("final");
  });

  it("adopts a session this client has never seen", () => {
    const merged = reconcileTabs({ all: [] }, [info({ id: "term_9", title: "Terminal 9" })]);
    expect(merged.all).toHaveLength(1);
    expect(merged.all[0]).toMatchObject({ id: "term_9", title: "Terminal 9", titleNumber: 1 });
    expect(merged.active).toBe("term_9");
  });

  it("gives adopted sessions distinct label numbers", () => {
    const merged = reconcileTabs(
      { all: [{ id: "term_1", title: "Terminal 1", titleNumber: 1 }] },
      [info({ id: "term_1" }), info({ id: "term_2" }), info({ id: "term_3" })],
    );
    expect(merged.all.map(tab => tab.titleNumber)).toEqual([1, 2, 3]);
  });

  it("falls back to the first tab when the stored active id is gone", () => {
    const merged = reconcileTabs(
      { active: "term_gone", all: [{ id: "term_1", title: "T", titleNumber: 1 }] },
      [info()],
    );
    expect(merged.active).toBe("term_1");
  });
});

describe("labels", () => {
  it("numbers default titles", () => {
    expect(defaultTitle(1)).toBe("Terminal 1");
    expect(defaultTitle(12)).toBe("Terminal 12");
  });
});
