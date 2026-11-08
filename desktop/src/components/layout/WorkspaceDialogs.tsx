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
 * The numbers are the backend's counts for the list the delete was started from,
 * and they answer the reader's real question — which conversations this click
 * removes, and where the survivors will be. The two lists are independent, so
 * the shelf's sentence never mentions the conversations the workspace list still
 * shows, and vice versa; that cross-talk is what the 115 build reported.
 */
function deleteDescription(
  state: WorkspaceDeleteDialogState | null,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  const summary = state?.summary;
  if (!summary)
    return t("workspaceDialogs.deleteDescription");
  const { deletedThreads: deleted, keptArchivedThreads: keptShelved, keptLiveThreads: keptLive } = summary;
  if (state?.scope === "archived") {
    if (deleted > 0 && keptLive > 0) {
      return t("workspaceDialogs.deleteShelvedWorkspaceDescription", {
        deleted,
        kept: keptLive,
      });
    }
    if (keptLive > 0)
      return t("workspaceDialogs.deleteShelvedKeepsLiveDescription", { kept: keptLive });
    if (deleted > 0)
      return t("workspaceDialogs.deleteShelvedOnlyDescription", { deleted });
    return t("workspaceDialogs.deleteActiveOnlyDescription");
  }
  if (deleted > 0 && keptShelved > 0) {
    return t("workspaceDialogs.deleteMixedDescription", {
      deleted,
      kept: keptShelved,
    });
  }
  if (keptShelved > 0)
    return t("workspaceDialogs.deleteKeptDescription", { kept: keptShelved });
  // Nothing survives: the delete takes everything in it, which is the sentence
  // this confirmation has always used.
  return t("workspaceDialogs.deleteActiveOnlyDescription");
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
