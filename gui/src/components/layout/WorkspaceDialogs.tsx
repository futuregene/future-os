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
        description={t("workspaceDialogs.deleteDescription")}
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
