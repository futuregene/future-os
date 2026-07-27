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

export interface BatchDeleteDialogState {
  error: string | null;
  submitting: boolean;
  deleteFiles: boolean;
  threads: StoredThread[];
  chatThreadCount: number;
  workspaceThreadCount: number;
}

interface AppShellDialogsProps {
  batchDeleteDialog: BatchDeleteDialogState | null;
  deleteDialog: DeleteDialogState | null;
  onConfirmBatchDeleteThread: () => void;
  onConfirmDeleteThread: () => void;
  onConfirmRenameThread: () => void;
  renameDialog: RenameDialogState | null;
  setBatchDeleteDialog: Dispatch<SetStateAction<BatchDeleteDialogState | null>>;
  setDeleteDialog: Dispatch<SetStateAction<DeleteDialogState | null>>;
  setRenameDialog: Dispatch<SetStateAction<RenameDialogState | null>>;
}

export function AppShellDialogs({
  batchDeleteDialog,
  deleteDialog,
  onConfirmBatchDeleteThread,
  onConfirmDeleteThread,
  onConfirmRenameThread,
  renameDialog,
  setBatchDeleteDialog,
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
      <ConfirmDeleteDialog
        description={batchDeleteDialog ? batchDeleteDescription(batchDeleteDialog, t) : undefined}
        error={batchDeleteDialog?.error ?? null}
        onClose={() => setBatchDeleteDialog(null)}
        onConfirm={onConfirmBatchDeleteThread}
        open={Boolean(batchDeleteDialog)}
        submitting={batchDeleteDialog?.submitting ?? false}
        title={t("appShellDialogs.batchDeleteTitle")}
      >
        {batchDeleteDialog
          ? (
              <div className="space-y-3">
                <div className="rounded-md border border-line-soft bg-surface-subtle p-3 text-sm text-ink">
                  <div className="mb-1 font-medium">{t("appShellDialogs.batchDeleteSummary", { count: batchDeleteDialog.threads.length })}</div>
                  <ul className="list-inside list-disc space-y-0.5 text-ink-soft">
                    {batchDeleteDialog.threads.slice(0, 5).map(thread => (
                      <li key={thread.id} className="truncate">{thread.title}</li>
                    ))}
                    {batchDeleteDialog.threads.length > 5
                      ? (
                          <li className="text-ink-muted">
                            {t("appShellDialogs.andMore", { count: batchDeleteDialog.threads.length - 5 })}
                          </li>
                        )
                      : null}
                  </ul>
                </div>
                {batchDeleteDialog.workspaceThreadCount > 0
                  ? (
                      <div className="rounded-md border border-line-soft bg-surface px-3 py-2 text-xs text-ink-soft">
                        {t("appShellDialogs.workspaceFilesUnaffected", { count: batchDeleteDialog.workspaceThreadCount })}
                      </div>
                    )
                  : null}
                {batchDeleteDialog.chatThreadCount > 0
                  ? (
                      <label className="flex cursor-pointer items-center gap-2 rounded-md border border-line-soft bg-surface px-3 py-2">
                        <input
                          checked={batchDeleteDialog.deleteFiles}
                          className="size-4 shrink-0 rounded border-line accent-accent"
                          disabled={batchDeleteDialog.submitting}
                          onChange={event =>
                            setBatchDeleteDialog(current =>
                              current ? { ...current, deleteFiles: event.target.checked } : current)}
                          type="checkbox"
                        />
                        <span className="text-sm text-ink-soft">
                          {t("appShellDialogs.deleteAssociatedFiles", { count: batchDeleteDialog.chatThreadCount })}
                        </span>
                      </label>
                    )
                  : null}
              </div>
            )
          : null}
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

function deleteThreadDescription(thread: StoredThread, t: (key: string, options?: Record<string, unknown>) => string) {
  if (thread.mode === "workspace") {
    return t("appShellDialogs.deleteWorkspaceDescription");
  }

  return t("appShellDialogs.deleteChatDescription");
}

function batchDeleteDescription(state: BatchDeleteDialogState, t: (key: string, options?: Record<string, unknown>) => string) {
  if (state.workspaceThreadCount > 0 && state.chatThreadCount > 0) {
    return t("appShellDialogs.batchDeleteMixedDescription", {
      chatCount: state.chatThreadCount,
      wsCount: state.workspaceThreadCount,
    });
  }
  if (state.workspaceThreadCount > 0) {
    return t("appShellDialogs.deleteWorkspaceDescription");
  }
  return t("appShellDialogs.deleteChatDescription");
}
