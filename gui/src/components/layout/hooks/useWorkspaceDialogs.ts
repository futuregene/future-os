import type { Dispatch, SetStateAction } from "react";
import type { StoredWorkspace } from "../../../integrations/storage/threadStore";
import { useState } from "react";
import i18n from "../../../i18n";
import { deleteWorkspace, renameWorkspace } from "../../../integrations/storage/threadStore";

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
 * mutation (which reconciles the active thread when its workspace is removed).
 */
export function useWorkspaceDialogs({
  refreshStore,
}: {
  refreshStore: (nextActiveThreadId?: string) => Promise<void>;
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
      setRenameDialog(null);
    }
    catch (error) {
      setRenameDialog(current =>
        current ? { ...current, error: error instanceof Error ? error.message : String(error), submitting: false } : current,
      );
    }
  }

  function openDelete(workspace: StoredWorkspace) {
    setDeleteDialog({ error: null, submitting: false, workspace });
  }

  async function confirmDelete() {
    if (!deleteDialog || deleteDialog.submitting)
      return;

    setDeleteDialog(current => current ? { ...current, error: null, submitting: true } : current);
    try {
      await deleteWorkspace(deleteDialog.workspace.id);
      await refreshStore();
      setDeleteDialog(null);
    }
    catch (error) {
      setDeleteDialog(current =>
        current ? { ...current, error: error instanceof Error ? error.message : String(error), submitting: false } : current,
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
