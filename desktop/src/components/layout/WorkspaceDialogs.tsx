import type { Dispatch, SetStateAction } from "react";
import type { WorkspaceDeleteDialogState, WorkspaceRenameDialogState } from "./hooks/useWorkspaceDialogs";
import { useTranslation } from "react-i18next";
import { ConfirmDeleteDialog, RenameDialog } from "./EntityDialogs";

interface WorkspaceDialogsProps {
  deleteDialog: WorkspaceDeleteDialogState | null;
  renameDialog: WorkspaceRenameDialogState | null;
  onConfirmDeleteWorkspace: () => void;
  onConfirmRenameWorkspace: () => void;
  setDeleteDialog: Dispatch<SetStateAction<WorkspaceDeleteDialogState | null>>;
  setRenameDialog: Dispatch<SetStateAction<WorkspaceRenameDialogState | null>>;
}

/**
 * What the workspace delete is about to do.
 *
 * The two numbers are the backend's count of the workspace's conversations, not
 * the loaded pages': deleting a workspace removes the conversations still in it
 * and keeps the ones the reader shelved (as independent chats). When only one
 * side has anything to say the other sentence is dropped, so a workspace with no
 * archive keeps the short confirmation it always had.
 */
function deleteDescription(
  state: WorkspaceDeleteDialogState | null,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  const summary = state?.summary;
  if (!summary)
    return t("workspaceDialogs.deleteDescription");
  if (summary.deletedThreads > 0 && summary.keptThreads > 0) {
    return t("workspaceDialogs.deleteMixedDescription", {
      deleted: summary.deletedThreads,
      kept: summary.keptThreads,
    });
  }
  if (summary.keptThreads > 0)
    return t("workspaceDialogs.deleteKeptDescription", { kept: summary.keptThreads });
  return t("workspaceDialogs.deleteDescription");
}

export function WorkspaceDialogs({
  deleteDialog,
  renameDialog,
  onConfirmDeleteWorkspace,
  onConfirmRenameWorkspace,
  setDeleteDialog,
  setRenameDialog,
}: WorkspaceDialogsProps) {
  const { t } = useTranslation("layout");
  return (
    <>
      <RenameDialog
        description={t("workspaceDialogs.renameDescription")}
        error={renameDialog?.error ?? null}
        label={t("appShellDialogs.nameLabel")}
        onChange={value =>
          setRenameDialog(current => current ? { ...current, error: null, value } : current)}
        onClose={() => setRenameDialog(null)}
        onConfirm={onConfirmRenameWorkspace}
        open={Boolean(renameDialog)}
        submitting={renameDialog?.submitting ?? false}
        title={t("workspaceDialogs.renameTitle")}
        value={renameDialog?.value ?? ""}
      />
      <ConfirmDeleteDialog
        description={deleteDescription(deleteDialog, t)}
        error={deleteDialog?.error ?? null}
        onClose={() => setDeleteDialog(null)}
        onConfirm={onConfirmDeleteWorkspace}
        open={Boolean(deleteDialog)}
        submitting={deleteDialog?.submitting ?? false}
        title={t("workspaceDialogs.deleteTitle")}
      >
        <div className="rounded-md border border-line-soft bg-surface-subtle p-3 text-sm text-ink">
          {deleteDialog?.workspace.name}
        </div>
      </ConfirmDeleteDialog>
    </>
  );
}
