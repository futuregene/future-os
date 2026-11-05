// @vitest-environment jsdom
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";

import { useThreadSearch } from "./useThreadSearch";

const searchThreads = vi.fn();

vi.mock("../../../integrations/storage/threadStore", () => ({
  searchThreads: (...args: unknown[]) => searchThreads(...args),
}));

function thread(id: string, title: string): StoredThread {
  return {
    agentSessionId: id,
    createdAt: 1,
    id,
    mode: "chat",
    pinned: false,
    readonly: false,
    status: "active",
    title,
    updatedAt: 1,
    workspaceId: "ws",
  } as unknown as StoredThread;
}

let query = "";

beforeEach(() => {
  query = "";
  searchThreads.mockReset();
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

/** Let the debounce fire and the promise chain settle. */
async function settle() {
  await act(async () => {
    vi.advanceTimersByTime(300);
  });
  await act(async () => {
    await Promise.resolve();
  });
}

function mount() {
  return renderHook(() => useThreadSearch(query));
}

/** Type a new query into the harness's box. */
async function type(next: string, harness: ReturnType<typeof mount>) {
  query = next;
  act(() => harness.rerender());
  await settle();
}

describe("useThreadSearch", () => {
  it("asks nothing while the box is empty", async () => {
    const h = mount();
    await settle();
    expect(searchThreads).not.toHaveBeenCalled();
    expect(h.current).toBeNull();
  });

  it("answers with the store's own rows, split the way the rail draws them", async () => {
    searchThreads.mockResolvedValue({ active: [thread("a", "Alpha")], archived: [thread("b", "Alpha shelved")] });
    query = "  Alpha  ";
    const h = mount();
    await settle();
    // The trimmed query is what the store is asked for, and what the answer is
    // filed under.
    expect(searchThreads).toHaveBeenCalledWith("Alpha", expect.any(Number));
    expect(h.current?.query).toBe("Alpha");
    expect(h.current?.active.map(item => item.id)).toEqual(["a"]);
    expect(h.current?.archived.map(item => item.id)).toEqual(["b"]);
  });

  it("waits out the typing before asking, and asks once", async () => {
    searchThreads.mockResolvedValue({ active: [], archived: [] });
    const h = mount();
    query = "a";
    act(() => h.rerender());
    query = "al";
    act(() => h.rerender());
    query = "alp";
    act(() => h.rerender());
    query = "alph";
    act(() => h.rerender());
    await settle();
    expect(searchThreads).toHaveBeenCalledTimes(1);
    expect(searchThreads).toHaveBeenCalledWith("alph", expect.any(Number));
  });

  it("drops an answer that is no longer the query on screen", async () => {
    let releaseFirst: (value: { active: StoredThread[]; archived: StoredThread[] }) => void = () => {};
    searchThreads
      .mockImplementationOnce(() => new Promise((resolve) => {
        releaseFirst = resolve;
      }))
      .mockResolvedValueOnce({ active: [thread("second", "Beta")], archived: [] });
    query = "Alpha";
    const h = mount();
    await settle();
    await type("Beta", h);
    expect(h.current?.active.map(item => item.id)).toEqual(["second"]);
    // The slow first answer lands last and must not overwrite the second.
    await act(async () => {
      releaseFirst({ active: [thread("first", "Alpha")], archived: [] });
      await Promise.resolve();
    });
    expect(h.current?.active.map(item => item.id)).toEqual(["second"]);
  });

  it("falls back to the list's own filter when the store cannot answer", async () => {
    searchThreads.mockRejectedValue(new Error("no store"));
    query = "Alpha";
    const h = mount();
    await settle();
    expect(h.current).toBeNull();
  });

  it("clears the answer when the query goes away", async () => {
    searchThreads.mockResolvedValue({ active: [thread("a", "Alpha")], archived: [] });
    query = "Alpha";
    const h = mount();
    await settle();
    expect(h.current).not.toBeNull();
    query = "";
    act(() => h.rerender());
    expect(h.current).toBeNull();
  });
});
