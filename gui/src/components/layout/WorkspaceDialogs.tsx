import type { Dispatch, SetStateAction } from "react";
import type { WorkspaceDeleteDialogState, WorkspaceRenameDialogState } from "./hooks/useWorkspaceDialogs";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

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
      <Dialog
        description={t("workspaceDialogs.renameDescription")}
        footer={(
          <>
            <Button
              disabled={renameDialog?.submitting}
              onClick={() => setRenameDialog(null)}
              type="button"
              variant="ghost"
            >
              {t("common:cancel")}
            </Button>
            <Button
              disabled={renameDialog?.submitting}
              onClick={() => onConfirmRenameWorkspace()}
              type="button"
              variant="primary"
            >
              {renameDialog?.submitting ? t("appShellDialogs.saving") : t("common:save")}
            </Button>
          </>
        )}
        onClose={() => setRenameDialog(null)}
        open={Boolean(renameDialog)}
        title={t("workspaceDialogs.renameTitle")}
      >
        <label className="block text-sm font-medium text-ink-soft" htmlFor="workspace-name">
          {t("appShellDialogs.nameLabel")}
        </label>
        <input
          autoFocus
          className="mt-2 h-10 w-full rounded-md border border-line bg-surface px-3 text-sm text-ink outline-none transition focus:border-focus focus:ring-2 focus:ring-focus"
          disabled={renameDialog?.submitting}
          id="workspace-name"
          onChange={event =>
            setRenameDialog(current => current ? { ...current, error: null, value: event.target.value } : current)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onConfirmRenameWorkspace();
            }
          }}
          value={renameDialog?.value ?? ""}
        />
        {renameDialog?.error ? <div className="mt-2 text-xs leading-5 text-danger">{renameDialog.error}</div> : null}
      </Dialog>
      <Dialog
        description={t("workspaceDialogs.deleteDescription")}
        footer={(
          <>
            <Button
              disabled={deleteDialog?.submitting}
              onClick={() => setDeleteDialog(null)}
              type="button"
              variant="ghost"
            >
              {t("common:cancel")}
            </Button>
            <Button
              disabled={deleteDialog?.submitting}
              onClick={() => onConfirmDeleteWorkspace()}
              type="button"
              variant="danger"
            >
              {deleteDialog?.submitting ? t("appShellDialogs.deleting") : t("common:delete")}
            </Button>
          </>
        )}
        onClose={() => setDeleteDialog(null)}
        open={Boolean(deleteDialog)}
        title={t("workspaceDialogs.deleteTitle")}
      >
        <div className="rounded-md border border-line-soft bg-surface-subtle p-3 text-sm text-ink">
          {deleteDialog?.workspace.name}
        </div>
        {deleteDialog?.error ? <div className="mt-2 text-xs leading-5 text-danger">{deleteDialog.error}</div> : null}
      </Dialog>
    </>
  );
}
