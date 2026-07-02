import type { Dispatch, SetStateAction } from "react";
import type { StoredThread, ThreadCleanupSummary } from "../../integrations/storage/threadStore";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

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
      <Dialog
        description={t("appShellDialogs.renameDescription")}
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
              onClick={() => onConfirmRenameThread()}
              type="button"
              variant="primary"
            >
              {renameDialog?.submitting ? t("appShellDialogs.saving") : t("common:save")}
            </Button>
          </>
        )}
        onClose={() => setRenameDialog(null)}
        open={Boolean(renameDialog)}
        title={t("appShellDialogs.renameTitle")}
      >
        <label className="block text-sm font-medium text-ink-soft" htmlFor="thread-title">
          {t("appShellDialogs.nameLabel")}
        </label>
        <input
          autoFocus
          className="mt-2 h-10 w-full rounded-md border border-line bg-surface px-3 text-sm text-ink outline-none transition focus:border-focus focus:ring-2 focus:ring-focus"
          disabled={renameDialog?.submitting}
          id="thread-title"
          onChange={event =>
            setRenameDialog(current => current ? { ...current, error: null, value: event.target.value } : current)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onConfirmRenameThread();
            }
          }}
          value={renameDialog?.value ?? ""}
        />
        {renameDialog?.error ? <div className="mt-2 text-xs leading-5 text-danger">{renameDialog.error}</div> : null}
      </Dialog>
      <Dialog
        description={deleteDialog ? deleteThreadDescription(deleteDialog.thread, t) : undefined}
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
              onClick={() => onConfirmDeleteThread()}
              type="button"
              variant="danger"
            >
              {deleteDialog?.submitting ? t("appShellDialogs.deleting") : t("common:delete")}
            </Button>
          </>
        )}
        onClose={() => setDeleteDialog(null)}
        open={Boolean(deleteDialog)}
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
        {deleteDialog?.error ? <div className="mt-2 text-xs leading-5 text-danger">{deleteDialog.error}</div> : null}
      </Dialog>
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
