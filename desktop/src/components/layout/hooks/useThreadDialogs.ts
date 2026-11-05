import type { Dispatch, SetStateAction } from "react";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import type { ArchiveDialogState, BatchArchiveDialogState, BatchDeleteDialogState, DeleteDialogState, RenameDialogState } from "../AppShellDialogs";
import type { RefreshStoreOptions } from "./useThreadStore";
import { useRef, useState } from "react";
import i18n, { getLanguage } from "../../../i18n";
import { invalidateAgentState, prefetchAgentState } from "../../../integrations/agent/agentStateCache";
import {
  archiveThreadTree,
  batchArchiveThreads,
  batchDeleteThreads,
  deleteThread,
  generateThreadTitle,
  getThreadCleanupSummary,
  listActiveThreads,
  renameThread,
  stopThreads,
} from "../../../integrations/storage/threadStore";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { treeMemberCount, treeMemberIds } from "../threadTree";

interface UseThreadDialogsParams {
  activeThreadId: string | null;
  refreshStore: (nextActiveThreadId?: string | null, options?: RefreshStoreOptions) => Promise<void>;
  /** Called after a successful archive so the shell can show its guide. */
  onArchived?: () => void;
  /** The store's rows, for the tree-member count a multi-member archive confirms. */
  threads?: StoredThread[];
}

export interface ThreadDialogs {
  renameDialog: RenameDialogState | null;
  deleteDialog: DeleteDialogState | null;
  batchDeleteDialog: BatchDeleteDialogState | null;
  archiveDialog: ArchiveDialogState | null;
  /**
   * A batch archive waiting on the reader's decision about running work.
   *
   * The selection can hold several trees; each one whose member is still
   * running has to stop before it can be shelved, so the batch asks once for
   * the whole set instead of dropping those rows with a toast afterwards.
   */
  batchArchiveDialog: BatchArchiveDialogState | null;
  setRenameDialog: Dispatch<SetStateAction<RenameDialogState | null>>;
  setDeleteDialog: Dispatch<SetStateAction<DeleteDialogState | null>>;
  setBatchDeleteDialog: Dispatch<SetStateAction<BatchDeleteDialogState | null>>;
  setArchiveDialog: Dispatch<SetStateAction<ArchiveDialogState | null>>;
  setBatchArchiveDialog: Dispatch<SetStateAction<BatchArchiveDialogState | null>>;
  openRename: (thread: StoredThread) => void;
  confirmRename: () => Promise<void>;
  generateTitle: () => Promise<void>;
  openDelete: (thread: StoredThread) => void;
  confirmDelete: () => Promise<void>;
  openBatchDelete: (threads: StoredThread[]) => void;
  confirmBatchDelete: () => Promise<void>;
  /**
   * Archive one conversation tree.
   *
   * A single-conversation tree archives straight away (the menu item is the
   * decision); a multi-member tree asks first, because a fork's children go with
   * it and that is not what "archive this conversation" sounds like.
   */
  requestArchive: (thread: StoredThread, treeMembers?: StoredThread[]) => Promise<void>;
  confirmArchive: () => Promise<void>;
  /** Archive the selected conversations, asking about any that are running. */
  archiveThreads: (threads: StoredThread[]) => Promise<void>;
  /** Stop the batch's running conversations, then archive it. */
  confirmBatchArchive: () => Promise<void>;
}

/**
 * Owns the rename/delete/archive confirmation dialogs for a thread: their
 * submitting/error state machines and, for chat deletes, the cleanup-summary
 * fetch. Refreshes the store after a successful mutation.
 */
export function useThreadDialogs({ activeThreadId, onArchived, refreshStore, threads = [] }: UseThreadDialogsParams): ThreadDialogs {
  const [deleteDialog, setDeleteDialog] = useState<DeleteDialogState | null>(null);
  const [renameDialog, setRenameDialog] = useState<RenameDialogState | null>(null);
  const [batchDeleteDialog, setBatchDeleteDialog] = useState<BatchDeleteDialogState | null>(null);
  const [archiveDialog, setArchiveDialog] = useState<ArchiveDialogState | null>(null);
  const [batchArchiveDialog, setBatchArchiveDialog] = useState<BatchArchiveDialogState | null>(null);
  const generationSequenceRef = useRef(0);
  const generatingDialogRef = useRef<RenameDialogState | null>(null);
  const threadsRef = useRef(threads);
  threadsRef.current = threads;

  function openRename(thread: StoredThread) {
    setRenameDialog({
      error: null,
      submitting: false,
      thread,
      value: thread.title,
    });
  }

  async function generateTitle() {
    if (!renameDialog || renameDialog.submitting || renameDialog.generating || generatingDialogRef.current === renameDialog)
      return;
    generatingDialogRef.current = renameDialog;
    const threadId = renameDialog.thread.id;
    const generation = ++generationSequenceRef.current;
    setRenameDialog(current => current ? { ...current, generating: true, generation, error: null } : current);
    try {
      const result = await generateThreadTitle(threadId, getLanguage());
      setRenameDialog(current => current?.generation === generation
        ? { ...current, value: result.title, generating: false }
        : current);
    }
    catch (error) {
      setRenameDialog(current => current?.generation === generation
        ? { ...current, error: errorMessage(error), generating: false }
        : current);
    }
    finally {
      if (generatingDialogRef.current === renameDialog)
        generatingDialogRef.current = null;
    }
  }

  async function confirmRename() {
    if (!renameDialog || renameDialog.submitting || renameDialog.generating)
      return;

    const nextTitle = renameDialog.value.trim();
    if (!nextTitle) {
      setRenameDialog(current => current ? { ...current, error: i18n.t("layout:threadDialogs.titleEmpty") } : current);
      return;
    }
    if (nextTitle === renameDialog.thread.title) {
      setRenameDialog(null);
      return;
    }

    setRenameDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      await renameThread({ threadId: renameDialog.thread.id, title: nextTitle });
      invalidateAgentState(renameDialog.thread.id);
      prefetchAgentState(renameDialog.thread.id);
      await refreshStore(renameDialog.thread.id);
      setRenameDialog(null);
    }
    catch (error) {
      setRenameDialog(current =>
        current
          ? {
              ...current,
              error: errorMessage(error),
              submitting: false,
            }
          : current,
      );
    }
  }

  function openDelete(thread: StoredThread) {
    setDeleteDialog({
      cleanupSummary: null,
      error: null,
      loadingSummary: thread.mode === "chat",
      submitting: false,
      thread,
    });

    if (thread.mode === "chat") {
      void getThreadCleanupSummary(thread.id)
        .then((summary) => {
          setDeleteDialog(current =>
            current?.thread.id === thread.id
              ? { ...current, cleanupSummary: summary, loadingSummary: false }
              : current,
          );
        })
        .catch(() => {
          setDeleteDialog(current =>
            current?.thread.id === thread.id
              ? { ...current, loadingSummary: false }
              : current,
          );
        });
    }
  }

  async function confirmDelete() {
    if (!deleteDialog || deleteDialog.submitting)
      return;

    const deleted = deleteDialog.thread;
    setDeleteDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      await deleteThread({ threadId: deleted.id, deleteFiles: false });
      // The row is gone for good, and the refresh has to be told so: pages no
      // longer returning it is otherwise the same evidence as "archived, still
      // open, read-only", so the store would hold it — and, when it was the open
      // one, keep drawing it while its sidebar row disappeared. Recomputing the
      // selection happens in the store, which is what knows which conversations
      // are left.
      await refreshStore(
        deleted.id === activeThreadId ? undefined : (activeThreadId ?? undefined),
        { removedThreadIds: [deleted.id] },
      );
      setDeleteDialog(null);
    }
    catch (error) {
      setDeleteDialog(current =>
        current
          ? {
              ...current,
              error: errorMessage(error),
              submitting: false,
            }
          : current,
      );
    }
  }

  function openBatchDelete(threads: StoredThread[]) {
    const chatThreads = threads.filter(t => t.mode === "chat");
    const workspaceThreads = threads.filter(t => t.mode === "workspace");
    setBatchDeleteDialog({
      error: null,
      submitting: false,
      deleteFiles: false,
      threads,
      chatThreadCount: chatThreads.length,
      workspaceThreadCount: workspaceThreads.length,
    });
  }

  async function confirmBatchDelete() {
    if (!batchDeleteDialog || batchDeleteDialog.submitting)
      return;

    setBatchDeleteDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      const threadIds = batchDeleteDialog.threads.map(t => t.id);
      await batchDeleteThreads({
        threadIds,
        deleteFiles: batchDeleteDialog.deleteFiles,
      });
      // Invalidate caches for all deleted threads.
      for (const threadId of threadIds) {
        invalidateAgentState(threadId);
      }
      // Same as a single delete: the rows are gone, not merely off the page. A
      // batch that included the open conversation must move the column on too.
      await refreshStore(undefined, { removedThreadIds: threadIds });
      setBatchDeleteDialog(null);
    }
    catch (error) {
      setBatchDeleteDialog(current =>
        current
          ? {
              ...current,
              error: errorMessage(error),
              submitting: false,
            }
          : current,
      );
    }
  }

  /**
   * A single conversation's archive.
   *
   * The tree is the unit, so the count of members decides the flow: one member
   * archives straight away (the menu item is the decision); several ask first,
   * because a fork's children go with it and that is not what "archive this
   * conversation" sounds like.
   *
   * A running member changes the question rather than answering "no": the row
   * menu cannot stop a run the reader has not opened, so the dialog offers the
   * two things they can do about it — cancel, or stop it and then archive. The
   * activity read is the backend's own (`listActiveThreads` covers runs this
   * store recorded *and* runs another client started); a failure to read it
   * falls through to the archive, whose refusal names the member.
   */
  async function requestArchive(thread: StoredThread, treeMembers?: StoredThread[]) {
    const lineage = treeMembers ?? threadsRef.current;
    const members = treeMemberCount(lineage, thread.id);
    const running = await runningMembers(treeMemberIds(lineage, thread.id), thread.id);
    if (running.length > 0) {
      setArchiveDialog({ error: null, members, running, submitting: false, thread });
      return;
    }
    if (members <= 1) {
      try {
        await archiveThreadTree(thread.id);
        onArchived?.();
        emitFutureEvent("archive-changed", undefined);
        // The tree left the active list. Saying so is what moves the column off
        // it when the open conversation was the one archived — the row is
        // otherwise held as "still open, just off this page", which drew a
        // shelved conversation as the current one under a sidebar row nobody
        // could see any more.
        await refreshStore(undefined, { archivedThreadIds: archivedIdsFor(lineage, thread.id) });
      }
      catch (error) {
        emitArchiveError(errorMessage(error));
      }
      return;
    }
    setArchiveDialog({ error: null, members, running: [], submitting: false, thread });
  }

  async function confirmArchive() {
    if (!archiveDialog || archiveDialog.submitting)
      return;
    const dialog = archiveDialog;
    setArchiveDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      // "Stop and archive": the reader chose the stop, so it happens before the
      // archive and its failure keeps the dialog open instead of shelving a
      // conversation whose run is still alive.
      if (dialog.running.length > 0)
        await stopThreads(dialog.running.map(thread => thread.id));
      await archiveThreadTree(dialog.thread.id);
      onArchived?.();
      emitFutureEvent("archive-changed", undefined);
      await refreshStore(undefined, {
        archivedThreadIds: archivedIdsFor(threadsRef.current, dialog.thread.id),
      });
      setArchiveDialog(null);
    }
    catch (error) {
      setArchiveDialog(current =>
        current
          ? { ...current, error: errorMessage(error), submitting: false }
          : current,
      );
    }
  }

  /** Archive a batch of trees; the backend skips the running ones and names them. */
  /**
   * Archive the selected conversations, asking first when any is still running.
   *
   * The batch is the one place a refusal used to arrive as a toast after the
   * fact, and the skipped rows then had to be archived one by one: the same two
   * choices the single-conversation archive offers — cancel, or stop and
   * archive — are asked here before anything is written.
   */
  async function archiveThreads(threads: StoredThread[]) {
    if (threads.length === 0)
      return;
    const running = await runningInTrees(threadsRef.current, threads);
    if (running.length > 0) {
      setBatchArchiveDialog({ error: null, running, submitting: false, threads });
      return;
    }
    await archiveThreadsNow(threads);
  }

  /** Stop the selected conversations' runs, then archive the batch. */
  async function confirmBatchArchive() {
    if (!batchArchiveDialog || batchArchiveDialog.submitting)
      return;
    const dialog = batchArchiveDialog;
    setBatchArchiveDialog(current =>
      current ? { ...current, error: null, submitting: true } : current);
    try {
      if (dialog.running.length > 0)
        await stopThreads(dialog.running.map(thread => thread.id));
      await archiveThreadsNow(dialog.threads);
      setBatchArchiveDialog(null);
    }
    catch (error) {
      setBatchArchiveDialog(current =>
        current
          ? { ...current, error: errorMessage(error), submitting: false }
          : current,
      );
    }
  }

  async function archiveThreadsNow(threads: StoredThread[]) {
    const wanted = threads.flatMap(thread => archivedIdsFor(threadsRef.current, thread.id));
    try {
      const result = await batchArchiveThreads(threads.map(thread => thread.id));
      onArchived?.();
      emitFutureEvent("archive-changed", undefined);
      // A tree a running member kept back is still on the active list, so it is
      // not reported as archived: the open conversation must not be moved off
      // one that stayed.
      const skipped = new Set(result.skipped.map(item => item.threadId));
      await refreshStore(undefined, {
        archivedThreadIds: wanted.filter(id => !skipped.has(id)),
      });
      if (result.skipped.length > 0) {
        emitArchiveSkipped(result.skipped);
      }
    }
    catch (error) {
      emitArchiveError(errorMessage(error));
    }
  }

  return {
    renameDialog,
    deleteDialog,
    batchDeleteDialog,
    archiveDialog,
    setRenameDialog,
    setDeleteDialog,
    setBatchDeleteDialog,
    setArchiveDialog,
    openRename,
    confirmRename,
    generateTitle,
    openDelete,
    confirmDelete,
    openBatchDelete,
    confirmBatchDelete,
    requestArchive,
    confirmArchive,
    archiveThreads,
    batchArchiveDialog,
    setBatchArchiveDialog,
    confirmBatchArchive,
  };
}

/**
 * The conversations in a scope that are still running.
 *
 * The backend's own activity read, not the rail's row statuses: an archive is
 * refused on runs this store recorded *and* on runs another client started
 * (the Agent's streaming set), and the dialog has to name exactly what the
 * refusal would. A scope the read cannot answer is treated as idle — the
 * archive call that follows still refuses anything alive.
 */
async function runningMembers(memberIds: string[], fallbackId: string): Promise<StoredThread[]> {
  const ids = new Set(memberIds);
  ids.add(fallbackId);
  try {
    const active = await listActiveThreads();
    return active.filter(thread => ids.has(thread.id));
  }
  catch {
    return [];
  }
}

/**
 * The running conversations among a selection, fork trees included.
 *
 * A tree is archived whole, so a selected root whose fork is running counts as
 * running: the batch would have to refuse it anyway.
 */
async function runningInTrees(rows: StoredThread[], threads: StoredThread[]): Promise<StoredThread[]> {
  const ids = new Set<string>();
  for (const thread of threads) {
    ids.add(thread.id);
    for (const id of treeMemberIds(rows, thread.id))
      ids.add(id);
  }
  try {
    const active = await listActiveThreads();
    return active.filter(thread => ids.has(thread.id));
  }
  catch {
    return [];
  }
}

/**
 * The conversation ids an archive of `threadId`'s tree takes off the active list.
 *
 * The rail's rows are the tree's source, but a row the store does not hold (a
 * stale rail row, a conversation archived from another surface) still archives
 * itself, so its own id is always part of the answer.
 */
function archivedIdsFor(rows: StoredThread[], threadId: string): string[] {
  const ids = treeMemberIds(rows, threadId);
  return ids.length > 0 ? ids : [threadId];
}

/** Surface an archive refusal as a toast (the rail's own affordance). */
function emitArchiveError(message: string) {
  window.dispatchEvent(new CustomEvent("futureos:toast", { detail: { message, tone: "error" } }));
}

/** Report the trees a running member kept back from a batch archive. */
function emitArchiveSkipped(skipped: Array<{ threadId: string; title: string }>) {
  const names = skipped.slice(0, 3).map(item => `"${item.title}"`).join(", ");
  const more = skipped.length - 3;
  window.dispatchEvent(new CustomEvent("futureos:toast", {
    detail: {
      message: i18n.t("layout:threadDialogs.archiveSkipped", {
        names,
        count: skipped.length,
        more: more > 0 ? ` +${more}` : "",
      }),
      tone: "info",
    },
  }));
}
