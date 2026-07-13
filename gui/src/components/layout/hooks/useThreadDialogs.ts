import type { Dispatch, SetStateAction } from "react";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import type { DeleteDialogState, RenameDialogState } from "../AppShellDialogs";
import { useState } from "react";
import i18n from "../../../i18n";
import { invalidateAgentState, prefetchAgentState } from "../../../integrations/agent/agentStateCache";
import { deleteThread, getThreadCleanupSummary, renameThread } from "../../../integrations/storage/threadStore";
import { errorMessage } from "../../../lib/errors";

interface UseThreadDialogsParams {
  activeThreadId: string | null;
  refreshStore: (nextActiveThreadId?: string) => Promise<void>;
}

export interface ThreadDialogs {
  renameDialog: RenameDialogState | null;
  deleteDialog: DeleteDialogState | null;
  setRenameDialog: Dispatch<SetStateAction<RenameDialogState | null>>;
  setDeleteDialog: Dispatch<SetStateAction<DeleteDialogState | null>>;
  openRename: (thread: StoredThread) => void;
  confirmRename: () => Promise<void>;
  openDelete: (thread: StoredThread) => void;
  confirmDelete: () => Promise<void>;
}

/**
 * Owns the rename/delete confirmation dialogs for a thread: their
 * submitting/error state machines and, for chat deletes, the cleanup-summary
 * fetch. Refreshes the store after a successful mutation.
 */
export function useThreadDialogs({ activeThreadId, refreshStore }: UseThreadDialogsParams): ThreadDialogs {
  const [deleteDialog, setDeleteDialog] = useState<DeleteDialogState | null>(null);
  const [renameDialog, setRenameDialog] = useState<RenameDialogState | null>(null);

  function openRename(thread: StoredThread) {
    setRenameDialog({
      error: null,
      submitting: false,
      thread,
      value: thread.title,
    });
  }

  async function confirmRename() {
    if (!renameDialog || renameDialog.submitting)
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
      await deleteThread(deleteDialog.thread.id);
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

  return {
    renameDialog,
    deleteDialog,
    setRenameDialog,
    setDeleteDialog,
    openRename,
    confirmRename,
    openDelete,
    confirmDelete,
  };
}
