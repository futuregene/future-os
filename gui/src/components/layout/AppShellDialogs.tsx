import type { Dispatch, SetStateAction } from "react";
import type { StoredThread, ThreadCleanupSummary } from "../../integrations/storage/threadStore";
import { useTranslation } from "react-i18next";
import { ConfirmDeleteDialog, RenameDialog } from "./EntityDialogs";

export interface DeleteDialogState {
  cleanupSummary: ThreadCleanupSummary | null;
  error: string | null;
  loadingSummary: boolean;
  submitting: boolean;
  thread: StoredThread;
}

export interface RenameDialogState {
  error: string | null;
  submitting: boolean;
  thread: StoredThread;
  value: string;
}

interface AppShellDialogsProps {
  deleteDialog: DeleteDialogState | null;
  onConfirmDeleteThread: () => void;
  onConfirmRenameThread: () => void;
  renameDialog: RenameDialogState | null;
  setDeleteDialog: Dispatch<SetStateAction<DeleteDialogState | null>>;
  setRenameDialog: Dispatch<SetStateAction<RenameDialogState | null>>;
}

export function AppShellDialogs({
  deleteDialog,
  onConfirmDeleteThread,
  onConfirmRenameThread,
  renameDialog,
  setDeleteDialog,
  setRenameDialog,
}: AppShellDialogsProps) {
  const { t } = useTranslation("layout");
  return (
    <>
      <RenameDialog
        description={t("appShellDialogs.renameDescription")}
        error={renameDialog?.error ?? null}
        label={t("appShellDialogs.nameLabel")}
        onChange={value =>
          setRenameDialog(current => current ? { ...current, error: null, value } : current)}
        onClose={() => setRenameDialog(null)}
        onConfirm={onConfirmRenameThread}
        open={Boolean(renameDialog)}
        submitting={renameDialog?.submitting ?? false}
        title={t("appShellDialogs.renameTitle")}
        value={renameDialog?.value ?? ""}
      />
      <ConfirmDeleteDialog
        description={deleteDialog ? deleteThreadDescription(deleteDialog.thread, t) : undefined}
        error={deleteDialog?.error ?? null}
        onClose={() => setDeleteDialog(null)}
        onConfirm={onConfirmDeleteThread}
        open={Boolean(deleteDialog)}
        submitting={deleteDialog?.submitting ?? false}
        title={t("appShellDialogs.deleteTitle")}
      >
        <div className="space-y-3">
          <div className="rounded-md border border-line-soft bg-surface-subtle p-3 text-sm text-ink">
            {deleteDialog?.thread.title}
          </div>
          {deleteDialog?.thread.mode === "chat" && deleteDialog.cleanupSummary && deleteDialog.cleanupSummary.artifactCount > 0
            ? (
                <ArtifactCount count={deleteDialog.cleanupSummary.artifactCount} />
              )
            : null}
        </div>
      </ConfirmDeleteDialog>
    </>
  );
}

function ArtifactCount({ count }: { count: number }) {
  const { t } = useTranslation("layout");
  return (
    <div className="flex items-center justify-between rounded-md border border-line-soft bg-surface px-3 py-2 text-sm">
      <span className="text-ink-soft">{t("appShellDialogs.artifacts")}</span>
      <span className="font-semibold text-ink">{count}</span>
    </div>
  );
}

function deleteThreadDescription(thread: StoredThread, t: (key: string) => string) {
  if (thread.mode === "workspace") {
    return t("appShellDialogs.deleteWorkspaceDescription");
  }

  return t("appShellDialogs.deleteChatDescription");
}
