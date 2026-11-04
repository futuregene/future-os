import type { Dispatch, SetStateAction } from "react";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import type { ArchiveDialogState, BatchDeleteDialogState, DeleteDialogState, RenameDialogState } from "../AppShellDialogs";
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
  renameThread,
} from "../../../integrations/storage/threadStore";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { treeMemberCount } from "../threadTree";

interface UseThreadDialogsParams {
  activeThreadId: string | null;
  refreshStore: (nextActiveThreadId?: string) => Promise<void>;
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
  setRenameDialog: Dispatch<SetStateAction<RenameDialogState | null>>;
  setDeleteDialog: Dispatch<SetStateAction<DeleteDialogState | null>>;
  setBatchDeleteDialog: Dispatch<SetStateAction<BatchDeleteDialogState | null>>;
  setArchiveDialog: Dispatch<SetStateAction<ArchiveDialogState | null>>;
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
  /** Archive the selected conversations, reporting the trees a run kept back. */
  archiveThreads: (threads: StoredThread[]) => Promise<void>;
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

    setDeleteDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      await deleteThread({ threadId: deleteDialog.thread.id, deleteFiles: false });
      await refreshStore(deleteDialog.thread.id === activeThreadId ? undefined : activeThreadId ?? undefined);
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
      await refreshStore();
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
   * archives immediately, several ask first (a confirmation that names the scope
   * is cheaper than a surprise restore of four rows later). A refusal — the tree
   * is running — comes back from the backend naming the member; it is surfaced
   * as a toast here rather than in a dialog, because the archive dialog may not
   * even be open (the single-member path never opens one).
   */
  async function requestArchive(thread: StoredThread, treeMembers?: StoredThread[]) {
    const lineage = treeMembers ?? threadsRef.current;
    const members = treeMemberCount(lineage, thread.id);
    if (members <= 1) {
      try {
        await archiveThreadTree(thread.id);
        onArchived?.();
        emitFutureEvent("archive-changed", undefined);
        await refreshStore();
      }
      catch (error) {
        emitArchiveError(errorMessage(error));
      }
      return;
    }
    setArchiveDialog({ error: null, submitting: false, thread, members });
  }

  async function confirmArchive() {
    if (!archiveDialog || archiveDialog.submitting)
      return;
    setArchiveDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      await archiveThreadTree(archiveDialog.thread.id);
      onArchived?.();
      emitFutureEvent("archive-changed", undefined);
      await refreshStore();
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
  async function archiveThreads(threads: StoredThread[]) {
    if (threads.length === 0)
      return;
    try {
      const result = await batchArchiveThreads(threads.map(thread => thread.id));
      onArchived?.();
      emitFutureEvent("archive-changed", undefined);
      await refreshStore();
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
  };
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
