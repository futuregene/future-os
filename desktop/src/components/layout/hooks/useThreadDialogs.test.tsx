// @vitest-environment jsdom
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useThreadDialogs } from "./useThreadDialogs";

const mocks = vi.hoisted(() => ({
  batchDelete: vi.fn(),
  deleteThread: vi.fn(),
  generateTitle: vi.fn(),
  cleanupSummary: vi.fn(),
  invalidate: vi.fn(),
  prefetch: vi.fn(),
  renameThread: vi.fn(),
  refreshStore: vi.fn(),
}));

vi.mock("../../../integrations/storage/threadStore", () => ({
  batchDeleteThreads: (...args: unknown[]) => mocks.batchDelete(...args),
  deleteThread: (...args: unknown[]) => mocks.deleteThread(...args),
  generateThreadTitle: (...args: unknown[]) => mocks.generateTitle(...args),
  getThreadCleanupSummary: (...args: unknown[]) => mocks.cleanupSummary(...args),
  renameThread: (...args: unknown[]) => mocks.renameThread(...args),
}));

vi.mock("../../../integrations/agent/agentStateCache", () => ({
  invalidateAgentState: (...args: unknown[]) => mocks.invalidate(...args),
  prefetchAgentState: (...args: unknown[]) => mocks.prefetch(...args),
}));

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: id,
    mode: "chat",
    workspaceId: `w-${id}`,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

function mount(activeThreadId: string | null = "t1") {
  const ref = { current: activeThreadId };
  const harness = renderHook(() => useThreadDialogs({
    activeThreadId: ref.current,
    refreshStore: mocks.refreshStore,
  }));
  return { harness, ref };
}

function deferred<T = { artifactCount: number }>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

describe("useThreadDialogs", () => {
  beforeEach(() => {
    mocks.batchDelete.mockReset().mockResolvedValue(undefined);
    mocks.deleteThread.mockReset().mockResolvedValue(undefined);
    mocks.generateTitle.mockReset().mockResolvedValue({ title: "Generated" });
    mocks.cleanupSummary.mockReset().mockResolvedValue({ artifactCount: 2 });
    mocks.invalidate.mockReset();
    mocks.prefetch.mockReset();
    mocks.renameThread.mockReset().mockResolvedValue(undefined);
    mocks.refreshStore.mockReset().mockResolvedValue(undefined);
  });

  it("seeds the rename dialog from the thread's current title", () => {
    const { harness } = mount();
    act(() => {
      harness.current.openRename(thread("t1", { title: "Old title" }));
    });

    expect(harness.current.renameDialog).toMatchObject({
      error: null,
      submitting: false,
      value: "Old title",
    });
    harness.unmount();
  });

  it("rejects a blank title without calling the store", async () => {
    const { harness } = mount();
    act(() => {
      harness.current.openRename(thread("t1", { title: "Old title" }));
    });
    act(() => {
      harness.current.setRenameDialog(current => (current ? { ...current, value: "   " } : current));
    });

    await act(async () => {
      await harness.current.confirmRename();
    });

    expect(mocks.renameThread).not.toHaveBeenCalled();
    expect(harness.current.renameDialog?.error).toBeTruthy();
    expect(harness.current.renameDialog?.submitting).toBe(false);
    harness.unmount();
  });

  it("closes without a round-trip when the title is unchanged", async () => {
    const { harness } = mount();
    act(() => {
      harness.current.openRename(thread("t1", { title: "Same" }));
    });

    await act(async () => {
      await harness.current.confirmRename();
    });

    expect(mocks.renameThread).not.toHaveBeenCalled();
    expect(harness.current.renameDialog).toBeNull();
    harness.unmount();
  });

  it("renames, invalidates the agent cache and refreshes onto the renamed thread", async () => {
    const { harness } = mount();
    act(() => {
      harness.current.openRename(thread("t1", { title: "Old" }));
    });
    act(() => {
      harness.current.setRenameDialog(current => (current ? { ...current, value: "  New  " } : current));
    });

    await act(async () => {
      await harness.current.confirmRename();
    });

    expect(mocks.renameThread).toHaveBeenCalledWith({ threadId: "t1", title: "New" });
    expect(mocks.invalidate).toHaveBeenCalledWith("t1");
    expect(mocks.prefetch).toHaveBeenCalledWith("t1");
    expect(mocks.refreshStore).toHaveBeenCalledWith("t1");
    expect(harness.current.renameDialog).toBeNull();
    harness.unmount();
  });

  it("shows a rename failure and leaves the dialog open for a retry", async () => {
    mocks.renameThread.mockRejectedValue(new Error("db locked"));
    const { harness } = mount();
    act(() => {
      harness.current.openRename(thread("t1", { title: "Old" }));
    });
    act(() => {
      harness.current.setRenameDialog(current => (current ? { ...current, value: "New" } : current));
    });

    await act(async () => {
      await harness.current.confirmRename();
    });

    expect(harness.current.renameDialog?.error).toBe("db locked");
    expect(harness.current.renameDialog?.submitting).toBe(false);
    expect(harness.current.renameDialog?.value).toBe("New");
    harness.unmount();
  });

  it("generates a title into the draft and surfaces a generation failure", async () => {
    const { harness } = mount();
    act(() => {
      harness.current.openRename(thread("t1", { title: "Old" }));
    });

    await act(async () => {
      await harness.current.generateTitle();
    });
    expect(harness.current.renameDialog?.value).toBe("Generated");
    expect(harness.current.renameDialog?.generating).toBe(false);

    mocks.generateTitle.mockRejectedValue(new Error("model offline"));
    await act(async () => {
      await harness.current.generateTitle();
    });
    expect(harness.current.renameDialog?.error).toBe("model offline");
    expect(harness.current.renameDialog?.generating).toBe(false);
    harness.unmount();
  });

  it("ignores a title generated for a dialog the user already replaced", async () => {
    let resolveOld!: (value: { title: string }) => void;
    mocks.generateTitle.mockReturnValueOnce(new Promise((resolve) => {
      resolveOld = resolve;
    }));

    const { harness } = mount();
    act(() => {
      harness.current.openRename(thread("t1", { title: "Old" }));
    });
    void act(() => {
      void harness.current.generateTitle();
    });

    // The user opens a different thread's rename dialog before the first
    // generation resolves.
    act(() => {
      harness.current.openRename(thread("t2", { title: "Other" }));
    });

    await act(async () => {
      resolveOld({ title: "Stale" });
      await Promise.resolve();
    });

    expect(harness.current.renameDialog?.thread.id).toBe("t2");
    expect(harness.current.renameDialog?.value).toBe("Other");
    harness.unmount();
  });

  it("ignores a generation failure for a dialog the user already dismissed", async () => {
    let rejectOld!: (error: Error) => void;
    mocks.generateTitle.mockReturnValueOnce(new Promise((_resolve, reject) => {
      rejectOld = reject;
    }));

    const { harness } = mount();
    act(() => {
      harness.current.openRename(thread("t1", { title: "Old" }));
    });
    void act(() => {
      void harness.current.generateTitle();
    });

    // The user closes the dialog while the model is still working.
    act(() => {
      harness.current.setRenameDialog(null);
    });
    expect(harness.current.renameDialog).toBeNull();

    // The failure then lands. It must not resurrect the dismissed dialog (and,
    // just as importantly, must not throw on `current?.generation`).
    await act(async () => {
      rejectOld(new Error("model offline"));
      await Promise.resolve();
    });
    expect(harness.current.renameDialog).toBeNull();
    harness.unmount();
  });

  it("does not generate twice for the same dialog while one is in flight", async () => {
    let resolveTitle!: (value: { title: string }) => void;
    mocks.generateTitle.mockReturnValue(new Promise((resolve) => {
      resolveTitle = resolve;
    }));

    const { harness } = mount();
    act(() => {
      harness.current.openRename(thread("t1", { title: "Old" }));
    });

    await act(async () => {
      const first = harness.current.generateTitle();
      await harness.current.generateTitle();
      resolveTitle({ title: "One" });
      await first;
    });

    expect(mocks.generateTitle).toHaveBeenCalledTimes(1);
    expect(harness.current.renameDialog?.value).toBe("One");
    harness.unmount();
  });

  it("loads the cleanup summary for a chat thread's delete dialog", async () => {
    const { harness } = mount();
    await act(async () => {
      harness.current.openDelete(thread("t1"));
      await Promise.resolve();
    });

    expect(mocks.cleanupSummary).toHaveBeenCalledWith("t1");
    expect(harness.current.deleteDialog?.cleanupSummary).toEqual({ artifactCount: 2 });
    expect(harness.current.deleteDialog?.loadingSummary).toBe(false);
    harness.unmount();
  });

  it("skips the cleanup probe for a workspace thread", async () => {
    const { harness } = mount();
    await act(async () => {
      harness.current.openDelete(thread("t1", { mode: "workspace" }));
      await Promise.resolve();
    });

    expect(mocks.cleanupSummary).not.toHaveBeenCalled();
    expect(harness.current.deleteDialog?.loadingSummary).toBe(false);
    harness.unmount();
  });

  it("stops the summary spinner when the cleanup probe fails", async () => {
    mocks.cleanupSummary.mockRejectedValue(new Error("no workspace"));
    const { harness } = mount();
    await act(async () => {
      harness.current.openDelete(thread("t1"));
      await Promise.resolve();
    });

    expect(harness.current.deleteDialog?.loadingSummary).toBe(false);
    expect(harness.current.deleteDialog?.cleanupSummary).toBeNull();
    harness.unmount();
  });

  it("ignores a cleanup summary that resolves after another thread's dialog opened", async () => {
    const slowFirst = deferred();
    const secondPending = deferred();
    mocks.cleanupSummary
      .mockReturnValueOnce(slowFirst.promise)
      .mockReturnValueOnce(secondPending.promise);

    const { harness } = mount();
    act(() => {
      harness.current.openDelete(thread("t1"));
    });
    act(() => {
      harness.current.openDelete(thread("t2"));
    });

    await act(async () => {
      slowFirst.resolve({ artifactCount: 9 });
      await Promise.resolve();
    });

    // The first thread's summary must not be written into the second thread's
    // dialog — the guard compares the dialog's thread id.
    expect(harness.current.deleteDialog?.thread.id).toBe("t2");
    expect(harness.current.deleteDialog?.cleanupSummary).toBeNull();
    // t2's own probe is still in flight, so its spinner stays up.
    expect(harness.current.deleteDialog?.loadingSummary).toBe(true);
    // Resolving it lands on t2's own dialog, proving the stale t1 result was
    // dropped rather than merely superseded.
    await act(async () => {
      secondPending.resolve({ artifactCount: 3 });
      await Promise.resolve();
    });
    expect(harness.current.deleteDialog?.cleanupSummary).toEqual({ artifactCount: 3 });
    harness.unmount();
  });

  it("deleting the active thread clears the selection", async () => {
    const { harness } = mount("t1");
    await act(async () => {
      harness.current.openDelete(thread("t1"));
      await Promise.resolve();
    });

    await act(async () => {
      await harness.current.confirmDelete();
    });

    expect(mocks.deleteThread).toHaveBeenCalledWith({ threadId: "t1", deleteFiles: false });
    // The active thread is gone — the store must pick a new one.
    expect(mocks.refreshStore).toHaveBeenCalledWith(undefined);
    expect(harness.current.deleteDialog).toBeNull();
    harness.unmount();
  });

  it("deleting a background thread keeps the active selection", async () => {
    const { harness } = mount("t1");
    await act(async () => {
      harness.current.openDelete(thread("t9"));
      await Promise.resolve();
    });

    await act(async () => {
      await harness.current.confirmDelete();
    });

    expect(mocks.refreshStore).toHaveBeenCalledWith("t1");
    harness.unmount();
  });

  it("keeps the delete dialog open with an error when deletion fails", async () => {
    mocks.deleteThread.mockRejectedValue(new Error("permission denied"));
    const { harness } = mount();
    await act(async () => {
      harness.current.openDelete(thread("t1"));
      await Promise.resolve();
    });

    await act(async () => {
      await harness.current.confirmDelete();
    });

    expect(harness.current.deleteDialog?.error).toBe("permission denied");
    expect(harness.current.deleteDialog?.submitting).toBe(false);
    harness.unmount();
  });

  it("counts chat and workspace threads when opening a batch delete", () => {
    const { harness } = mount();
    act(() => {
      harness.current.openBatchDelete([
        thread("a"),
        thread("b"),
        thread("c", { mode: "workspace" }),
      ]);
    });

    expect(harness.current.batchDeleteDialog).toMatchObject({
      chatThreadCount: 2,
      workspaceThreadCount: 1,
      deleteFiles: false,
      error: null,
      submitting: false,
    });
    harness.unmount();
  });

  it("batch-deletes, invalidates every removed thread's cache and clears the selection", async () => {
    const { harness } = mount("t1");
    act(() => {
      harness.current.openBatchDelete([thread("a"), thread("b")]);
    });
    act(() => {
      harness.current.setBatchDeleteDialog(current => (current ? { ...current, deleteFiles: true } : current));
    });

    await act(async () => {
      await harness.current.confirmBatchDelete();
    });

    expect(mocks.batchDelete).toHaveBeenCalledWith({ threadIds: ["a", "b"], deleteFiles: true });
    expect(mocks.invalidate.mock.calls.map(([id]) => id)).toEqual(["a", "b"]);
    expect(mocks.refreshStore).toHaveBeenCalledWith();
    expect(harness.current.batchDeleteDialog).toBeNull();
    harness.unmount();
  });

  it("keeps the batch dialog open with an error when the batch delete fails", async () => {
    mocks.batchDelete.mockRejectedValue(new Error("locked"));
    const { harness } = mount();
    act(() => {
      harness.current.openBatchDelete([thread("a")]);
    });

    await act(async () => {
      await harness.current.confirmBatchDelete();
    });

    expect(harness.current.batchDeleteDialog?.error).toBe("locked");
    expect(mocks.invalidate).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("confirming with no dialog open is a no-op", async () => {
    const { harness } = mount();
    await act(async () => {
      await harness.current.confirmRename();
      await harness.current.confirmDelete();
      await harness.current.confirmBatchDelete();
      await harness.current.generateTitle();
    });

    expect(mocks.renameThread).not.toHaveBeenCalled();
    expect(mocks.deleteThread).not.toHaveBeenCalled();
    expect(mocks.batchDelete).not.toHaveBeenCalled();
    expect(mocks.generateTitle).not.toHaveBeenCalled();
    harness.unmount();
  });

  // ── dismissed/replaced while the work was in flight ─────────────────────────
  // Every `setState(current => current ? … : current)` below has a null side
  // that only runs when the user closed (or replaced) the dialog before the
  // updater was applied. Without these, a late result could reopen a dialog the
  // user had already dismissed, or overwrite a different dialog's state.

  it("does not reopen a dismissed dialog when a blank title is rejected", async () => {
    const { harness } = mount();
    act(() => harness.current.openRename(thread("t1", { title: "Old" })));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "   " } : state)));

    const confirm = harness.current.confirmRename;
    const close = harness.current.setRenameDialog;
    await act(async () => {
      close(null);
      await confirm();
    });
    expect(harness.current.renameDialog).toBeNull();
    expect(mocks.renameThread).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("does not resurrect a dismissed dialog when a valid rename starts", async () => {
    const { harness } = mount();
    act(() => harness.current.openRename(thread("t1", { title: "Old" })));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "New" } : state)));

    const confirm = harness.current.confirmRename;
    const close = harness.current.setRenameDialog;
    await act(async () => {
      close(null);
      await confirm();
    });
    expect(mocks.renameThread).toHaveBeenCalledWith({ threadId: "t1", title: "New" });
    expect(harness.current.renameDialog).toBeNull();
    harness.unmount();
  });

  it("does not reopen a dismissed dialog when the rename write fails late", async () => {
    let failWrite!: (error: Error) => void;
    mocks.renameThread.mockReturnValue(new Promise<void>((_resolve, reject) => {
      failWrite = reject;
    }));
    const { harness } = mount();
    act(() => harness.current.openRename(thread("t1", { title: "Old" })));
    act(() => harness.current.setRenameDialog(state => (state ? { ...state, value: "New" } : state)));
    await act(async () => {
      void harness.current.confirmRename();
      await Promise.resolve();
    });

    act(() => harness.current.setRenameDialog(null));
    await act(async () => {
      failWrite(new Error("db locked"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.renameDialog).toBeNull();
    harness.unmount();
  });

  it("does not start a title generation on a dialog the user dismissed", async () => {
    const { harness } = mount();
    act(() => harness.current.openRename(thread("t1", { title: "Old" })));

    const generate = harness.current.generateTitle;
    const close = harness.current.setRenameDialog;
    await act(async () => {
      close(null);
      await generate();
    });
    expect(mocks.generateTitle).toHaveBeenCalledTimes(1);
    expect(harness.current.renameDialog).toBeNull();
    harness.unmount();
  });

  it("drops a generated title whose dialog generation no longer matches", async () => {
    const pending = deferred<{ title: string }>();
    mocks.generateTitle.mockReturnValueOnce(pending.promise);
    const { harness } = mount();
    act(() => harness.current.openRename(thread("t1", { title: "Old" })));
    await act(async () => {
      void harness.current.generateTitle();
      await Promise.resolve();
    });

    // The dialog is replaced while the model is still writing: the reply must
    // land on nothing rather than on the newer dialog.
    act(() => {
      harness.current.openRename(thread("t2", { title: "Other" }));
    });
    await act(async () => {
      pending.resolve({ title: "Stale" });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.renameDialog?.thread.id).toBe("t2");
    expect(harness.current.renameDialog?.value).toBe("Other");
    expect(harness.current.renameDialog?.generating).toBeFalsy();
    harness.unmount();
  });

  it("does not reopen a dismissed dialog when the delete write fails late", async () => {
    let failWrite!: (error: Error) => void;
    mocks.deleteThread.mockReturnValue(new Promise<void>((_resolve, reject) => {
      failWrite = reject;
    }));
    const { harness } = mount();
    await act(async () => {
      harness.current.openDelete(thread("t1"));
      await Promise.resolve();
    });
    await act(async () => {
      void harness.current.confirmDelete();
      await Promise.resolve();
    });

    act(() => harness.current.setDeleteDialog(null));
    await act(async () => {
      failWrite(new Error("permission denied"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.deleteDialog).toBeNull();
    harness.unmount();
  });

  it("does not resurrect a dismissed delete dialog when the delete starts", async () => {
    const { harness } = mount();
    await act(async () => {
      harness.current.openDelete(thread("t1"));
      await Promise.resolve();
    });

    const confirm = harness.current.confirmDelete;
    const close = harness.current.setDeleteDialog;
    await act(async () => {
      close(null);
      await confirm();
    });
    expect(mocks.deleteThread).toHaveBeenCalledWith({ deleteFiles: false, threadId: "t1" });
    expect(harness.current.deleteDialog).toBeNull();
    harness.unmount();
  });

  it("clears the selection when the thread deleted with no active thread was not active anyway", async () => {
    // `activeThreadId` is null, so the refresh target is the `?? undefined`
    // fallback rather than the id: there is no selection to preserve.
    const { harness } = mount(null);
    await act(async () => {
      harness.current.openDelete(thread("t9"));
      await Promise.resolve();
    });
    await act(async () => {
      await harness.current.confirmDelete();
    });
    expect(mocks.refreshStore).toHaveBeenCalledWith(undefined);
    harness.unmount();
  });

  it("does not resurrect a dismissed dialog when the cleanup probe rejects", async () => {
    // The catch side of the same guard: the probe fails after the dialog was
    // dismissed, so the spinner must not reappear on a dialog that is gone.
    let failProbe!: (error: Error) => void;
    mocks.cleanupSummary.mockReturnValueOnce(new Promise((_resolve, reject) => {
      failProbe = reject;
    }));
    const { harness } = mount();
    act(() => harness.current.openDelete(thread("t1")));
    act(() => harness.current.setDeleteDialog(null));
    await act(async () => {
      failProbe(new Error("no workspace"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.deleteDialog).toBeNull();
    harness.unmount();
  });

  it("keeps a newer dialog's generation marker when an older generation finishes", async () => {
    const first = deferred<{ title: string }>();
    const second = deferred<{ title: string }>();
    mocks.generateTitle.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    const { harness } = mount();
    act(() => harness.current.openRename(thread("t1", { title: "One" })));
    await act(async () => {
      void harness.current.generateTitle();
      await Promise.resolve();
    });

    // A second dialog starts its own generation while the first is in flight.
    act(() => harness.current.openRename(thread("t2", { title: "Two" })));
    await act(async () => {
      void harness.current.generateTitle();
      await Promise.resolve();
    });

    // The first dialog's generation lands. It belongs to the older dialog, so it
    // must neither write its title anywhere nor clear the newer dialog's marker.
    await act(async () => {
      first.resolve({ title: "One done" });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.renameDialog?.thread.id).toBe("t2");
    expect(harness.current.renameDialog?.value).toBe("Two");

    // Because its marker survived, the newer dialog's own generation still lands.
    await act(async () => {
      second.resolve({ title: "Two done" });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.renameDialog?.value).toBe("Two done");
    expect(harness.current.renameDialog?.generating).toBe(false);
    harness.unmount();
  });

  it("does not resurrect a dismissed dialog when the cleanup probe resolves", async () => {
    const slow = deferred<{ artifactCount: number }>();
    mocks.cleanupSummary.mockReturnValueOnce(slow.promise);
    const { harness } = mount();
    act(() => harness.current.openDelete(thread("t1")));

    // Dismissed while the probe is still running: its result must land on
    // nothing rather than reopening the dialog the user just closed.
    act(() => harness.current.setDeleteDialog(null));
    await act(async () => {
      slow.resolve({ artifactCount: 9 });
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.deleteDialog).toBeNull();
    harness.unmount();
  });

  it("does not reopen a dismissed dialog when the batch delete fails late", async () => {
    let failWrite!: (error: Error) => void;
    mocks.batchDelete.mockReturnValue(new Promise<void>((_resolve, reject) => {
      failWrite = reject;
    }));
    const { harness } = mount();
    act(() => harness.current.openBatchDelete([thread("a"), thread("b")]));
    await act(async () => {
      void harness.current.confirmBatchDelete();
      await Promise.resolve();
    });

    act(() => harness.current.setBatchDeleteDialog(null));
    await act(async () => {
      failWrite(new Error("locked"));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(harness.current.batchDeleteDialog).toBeNull();
    harness.unmount();
  });

  it("does not resurrect a dismissed batch dialog when the delete starts", async () => {
    const { harness } = mount();
    act(() => harness.current.openBatchDelete([thread("a")]));

    const confirm = harness.current.confirmBatchDelete;
    const close = harness.current.setBatchDeleteDialog;
    await act(async () => {
      close(null);
      await confirm();
    });
    expect(mocks.batchDelete).toHaveBeenCalledTimes(1);
    expect(harness.current.batchDeleteDialog).toBeNull();
    harness.unmount();
  });
});
