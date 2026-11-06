import type { Dispatch, SetStateAction } from "react";
import type { WorkspaceDeleteSummary } from "../../../integrations/storage/threads";
import type { StoredWorkspace } from "../../../integrations/storage/threadStore";
import type { RefreshStoreOptions } from "./useThreadStore";
import { useState } from "react";
import i18n from "../../../i18n";
import { deleteWorkspace, renameWorkspace, workspaceDeleteSummary } from "../../../integrations/storage/threadStore";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";

export interface WorkspaceRenameDialogState {
  workspace: StoredWorkspace;
  value: string;
  error: string | null;
  submitting: boolean;
}

export interface WorkspaceDeleteDialogState {
  workspace: StoredWorkspace;
  error: string | null;
  submitting: boolean;
  /**
   * How many conversations the delete removes, and how many it keeps.
   *
   * Read when the dialog opens (the counts are the backend's, not the loaded
   * pages'), so the confirmation can say what will happen before the reader
   * commits. Null while it is being read, or when the read failed — the dialog
   * then falls back to its general sentence.
   */
  summary: WorkspaceDeleteSummary | null;
  loadingSummary: boolean;
}

export interface WorkspaceDialogs {
  renameDialog: WorkspaceRenameDialogState | null;
  deleteDialog: WorkspaceDeleteDialogState | null;
  setRenameDialog: Dispatch<SetStateAction<WorkspaceRenameDialogState | null>>;
  setDeleteDialog: Dispatch<SetStateAction<WorkspaceDeleteDialogState | null>>;
  openRename: (workspace: StoredWorkspace) => void;
  confirmRename: () => Promise<void>;
  openDelete: (workspace: StoredWorkspace) => void;
  confirmDelete: () => Promise<void>;
}

/**
 * Owns the rename/delete confirmation dialogs for a workspace: their
 * submitting/error state machines. Refreshes the store after a successful
 * mutation, and a delete names the workspace so the store takes its
 * conversations with it (the active-thread reconciliation is the store's).
 */
export function useWorkspaceDialogs({
  refreshStore,
}: {
  refreshStore: (nextActiveThreadId?: string | null, options?: RefreshStoreOptions) => Promise<void>;
}): WorkspaceDialogs {
  const [renameDialog, setRenameDialog] = useState<WorkspaceRenameDialogState | null>(null);
  const [deleteDialog, setDeleteDialog] = useState<WorkspaceDeleteDialogState | null>(null);

  function openRename(workspace: StoredWorkspace) {
    setRenameDialog({ error: null, submitting: false, value: workspace.name, workspace });
  }

  async function confirmRename() {
    if (!renameDialog || renameDialog.submitting)
      return;

    const nextName = renameDialog.value.trim();
    if (!nextName) {
      setRenameDialog(current => current ? { ...current, error: i18n.t("layout:workspaceDialogs.nameEmpty") } : current);
      return;
    }
    if (nextName === renameDialog.workspace.name) {
      setRenameDialog(null);
      return;
    }

    setRenameDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      await renameWorkspace({ name: nextName, workspaceId: renameDialog.workspace.id });
      await refreshStore();
      // A shelved workspace's heading is drawn from the archived view's own
      // list, which the store's pages never hold: without this the rename landed
      // in the database and the heading kept the old name until the view was
      // reloaded. The same event archive/restore/delete already fire.
      emitFutureEvent("archive-changed", undefined);
      setRenameDialog(null);
    }
    catch (error) {
      setRenameDialog(current =>
        current ? { ...current, error: errorMessage(error), submitting: false } : current,
      );
    }
  }

  function openDelete(workspace: StoredWorkspace) {
    setDeleteDialog({
      error: null,
      loadingSummary: true,
      submitting: false,
      summary: null,
      workspace,
    });
    void workspaceDeleteSummary(workspace.id)
      .then((summary) => {
        setDeleteDialog(current =>
          current?.workspace.id === workspace.id
            ? { ...current, loadingSummary: false, summary }
            : current,
        );
      })
      .catch(() => {
        setDeleteDialog(current =>
          current?.workspace.id === workspace.id
            ? { ...current, loadingSummary: false }
            : current,
        );
      });
  }

  async function confirmDelete() {
    if (!deleteDialog || deleteDialog.submitting)
      return;

    setDeleteDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      const { workspace } = deleteDialog;
      await deleteWorkspace(workspace.id);
      // Every conversation inside went with it, and the store cannot tell that
      // from "not on this page": the open one would stay on screen as a row no
      // list will ever return (see `RefreshStoreOptions.removedWorkspaceIds`).
      await refreshStore(undefined, { removedWorkspaceIds: [workspace.id] });
      // A delete is not an archive delete: the conversations shelved out of the
      // workspace survive as ordinary chats, so the archived view holds rows the
      // store's active pages never saw. It re-reads on this event, which also
      // drops the shelf of any workspace that is really gone.
      emitFutureEvent("archive-changed", undefined);
      setDeleteDialog(null);
    }
    catch (error) {
      setDeleteDialog(current =>
        current ? { ...current, error: errorMessage(error), submitting: false } : current,
      );
    }
  }

  return {
    confirmDelete,
    confirmRename,
    deleteDialog,
    openDelete,
    openRename,
    renameDialog,
    setDeleteDialog,
    setRenameDialog,
  };
}
