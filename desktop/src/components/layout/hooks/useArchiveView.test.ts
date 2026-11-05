// @vitest-environment jsdom
import type { StoredThread, StoredWorkspace } from "../../../integrations/storage/threadStore";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";

import { useArchiveView } from "./useArchiveView";

const listThreadsPage = vi.fn();
const listWorkspacesPage = vi.fn();

vi.mock("../../../integrations/storage/threadStore", () => ({
  listThreadsPage: (...args: unknown[]) => listThreadsPage(...args),
  listWorkspacesPage: (...args: unknown[]) => listWorkspacesPage(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: () => Promise.resolve(() => {}),
}));

function thread(id: string): StoredThread {
  return { id, title: id, status: "archived", mode: "chat" } as unknown as StoredThread;
}

function pages(ids: string[]) {
  return ids.map(id => thread(id));
}

function page(rows: StoredThread[], nextCursor: string | null, totalRoots: number) {
  return { threads: rows, nextCursor, totalRoots };
}

beforeEach(() => {
  listThreadsPage.mockReset();
  listWorkspacesPage.mockReset();
  listWorkspacesPage.mockResolvedValue({ workspaces: [] as StoredWorkspace[], nextCursor: null });
});

async function mountArchive() {
  const h = renderHook(() => useArchiveView());
  act(() => h.current.ensureLoaded());
  await flushAsync();
  await flushAsync();
  return h;
}

/**
 * The archived lists say the same sentence as the active ones.
 *
 * Their controls name the stream's real remainder, so one click has to load all
 * of it: a page per click after naming "the other 13" is the same lie the recent
 * view's control used to tell.
 */
describe("useArchiveView's paging", () => {
  it("counts the archived stream's remainder and walks it to the end", async () => {
    const all = Array.from({ length: 23 }, (_, index) => `a${index}`);
    listThreadsPage
      .mockResolvedValueOnce(page(pages(all.slice(0, 10)), "c2", 23))
      .mockResolvedValueOnce(page(pages(all.slice(10, 20)), "c3", 23))
      .mockResolvedValueOnce(page(pages(all.slice(20)), null, 23));

    const h = await mountArchive();
    expect(listThreadsPage.mock.calls[0]![1]).toBeNull();
    expect(h.current.threads).toHaveLength(10);
    expect(h.current.threadsRemaining).toBe(13);
    expect(h.current.hasMoreThreads).toBe(true);

    await act(async () => {
      await h.current.loadAllThreads();
    });

    // Both remaining pages, in order, and nothing left to offer.
    expect(listThreadsPage.mock.calls.map(call => call[1])).toEqual([null, "c2", "c3"]);
    expect(h.current.threads).toHaveLength(23);
    expect(h.current.threadsRemaining).toBe(0);
    expect(h.current.hasMoreThreads).toBe(false);
    h.unmount();
  });

  it("does the same for one shelved workspace's contents", async () => {
    listThreadsPage.mockResolvedValueOnce(page([], null, 0));
    const h = await mountArchive();

    const inWorkspace = Array.from({ length: 12 }, (_, index) => `w${index}`);
    listThreadsPage
      .mockResolvedValueOnce(page(pages(inWorkspace.slice(0, 10)), "w-cursor", 12))
      .mockResolvedValueOnce(page(pages(inWorkspace.slice(10)), null, 12));
    act(() => h.current.ensureWorkspaceThreads("w-shelved"));
    await flushAsync();
    await flushAsync();
    expect(h.current.workspaceThreads["w-shelved"]).toHaveLength(10);
    // Twelve conversations, ten on screen: the group's control says two.
    expect(h.current.workspaceRemaining["w-shelved"]).toBe(2);

    await act(async () => {
      await h.current.loadAllWorkspaceThreads("w-shelved");
    });
    expect(h.current.workspaceThreads["w-shelved"]).toHaveLength(12);
    expect(h.current.workspaceRemaining["w-shelved"]).toBe(0);
    h.unmount();
  });

  it("fetches a group's contents once while the first request is in flight", async () => {
    listThreadsPage.mockResolvedValueOnce(page([], null, 0));
    const h = await mountArchive();

    let resolve!: (value: ReturnType<typeof page>) => void;
    const pending = new Promise<ReturnType<typeof page>>((done) => {
      resolve = done;
    });
    listThreadsPage.mockReturnValueOnce(pending);
    // The view asks again on every render while an expanded group has no page;
    // that must be one request, not one per render.
    act(() => {
      h.current.ensureWorkspaceThreads("w1");
      h.current.ensureWorkspaceThreads("w1");
    });
    expect(listThreadsPage).toHaveBeenCalledTimes(2);

    act(() => resolve(page(pages(["one"]), null, 1)));
    await flushAsync();
    await flushAsync();
    expect(h.current.workspaceThreads.w1?.map(row => row.id)).toEqual(["one"]);
    // Loaded: another ask is a no-op.
    act(() => h.current.ensureWorkspaceThreads("w1"));
    expect(listThreadsPage).toHaveBeenCalledTimes(2);
    h.unmount();
  });

  it("re-reads an open group's contents when the shelf refreshes", async () => {
    listThreadsPage
      // The entry's own read, then the group the reader expands.
      .mockResolvedValueOnce(page([], null, 0))
      .mockResolvedValueOnce(page(pages(["one", "restored"]), null, 2))
      // The refresh after a restore: the stream, then the group again.
      .mockResolvedValueOnce(page([], null, 0))
      .mockResolvedValueOnce(page(pages(["one"]), null, 1));
    listWorkspacesPage
      .mockResolvedValueOnce({ nextCursor: null, workspaces: [{ id: "w1", name: "A" } as StoredWorkspace] })
      .mockResolvedValueOnce({ nextCursor: null, workspaces: [{ id: "w1", name: "A" } as StoredWorkspace] });

    const h = await mountArchive();
    act(() => h.current.ensureWorkspaceThreads("w1"));
    await flushAsync();
    await flushAsync();
    expect(h.current.workspaceThreads.w1?.map(row => row.id)).toEqual(["one", "restored"]);

    // One conversation was restored: the workspace stays on the shelf, but its
    // cached page must not keep drawing the row that left it.
    await act(async () => {
      await h.current.refresh();
    });
    expect(h.current.workspaceThreads.w1?.map(row => row.id)).toEqual(["one"]);
    h.unmount();
  });
});
