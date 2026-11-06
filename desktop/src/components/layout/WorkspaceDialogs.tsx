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
 * The three numbers are the backend's counts, not the loaded pages', and they
 * answer the reader's real question — which conversations survive, and where
 * they will be. Neither sentence may call a conversation "archived" unless the
 * reader shelved it themselves: a conversation hidden by its folder was never
 * archived, and saying so read as a mistake (115 build, "ces codex").
 */
function deleteDescription(
  state: WorkspaceDeleteDialogState | null,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  const summary = state?.summary;
  if (!summary)
    return t("workspaceDialogs.deleteDescription");
  const { deletedThreads: deleted, keptArchivedThreads: keptShelved, keptLiveThreads: keptLive } = summary;
  if (deleted > 0 && keptShelved > 0) {
    return t("workspaceDialogs.deleteMixedDescription", {
      deleted,
      kept: keptShelved,
    });
  }
  if (deleted > 0 && keptLive > 0) {
    // Deleting from the archived shelf: the folder hid conversations the reader
    // never shelved, and they come back to the conversations list.
    return t("workspaceDialogs.deleteShelvedWorkspaceDescription", {
      deleted,
      kept: keptLive,
    });
  }
  if (keptShelved > 0)
    return t("workspaceDialogs.deleteKeptDescription", { kept: keptShelved });
  if (keptLive > 0)
    return t("workspaceDialogs.deleteShelvedKeepsLiveDescription", { kept: keptLive });
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
