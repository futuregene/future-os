import type { Dispatch, SetStateAction } from "react";
import type { StoredThread, StoredWorkspace, ThreadCleanupSummary } from "../../integrations/storage/threadStore";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { ConfirmDeleteDialog, RenameDialog } from "./EntityDialogs";

export interface DeleteDialogState {
  cleanupSummary: ThreadCleanupSummary | null;
  error: string | null;
  loadingSummary: boolean;
  submitting: boolean;
  thread: StoredThread;
}

export interface RenameDialogState {
  generation?: number;
  generating?: boolean;
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

/** Confirm shelving one fork tree; `members` is how many conversations go. */
export interface ArchiveDialogState {
  error: string | null;
  submitting: boolean;
  thread: StoredThread;
  members: number;
  /**
   * Conversations in the scope that are still running.
   *
   * Empty means the archive is a plain confirmation; non-empty turns the
   * confirm into "stop and archive", because a run has to end before a
   * conversation can be shelved.
   */
  running: StoredThread[];
}

/** Confirm shelving a workspace whose conversations are still running. */
export interface ArchiveWorkspaceDialogState {
  error: string | null;
  submitting: boolean;
  workspace: StoredWorkspace;
  running: StoredThread[];
}

interface AppShellDialogsProps {
  archiveDialog: ArchiveDialogState | null;
  archiveWorkspaceDialog: ArchiveWorkspaceDialogState | null;
  batchDeleteDialog: BatchDeleteDialogState | null;
  deleteDialog: DeleteDialogState | null;
  onConfirmArchiveThread: () => void;
  onConfirmArchiveWorkspace: () => void;
  onConfirmBatchDeleteThread: () => void;
  onConfirmDeleteThread: () => void;
  onConfirmRenameThread: () => void;
  onGenerateTitle: () => void;
  renameDialog: RenameDialogState | null;
  setArchiveDialog: Dispatch<SetStateAction<ArchiveDialogState | null>>;
  setArchiveWorkspaceDialog: Dispatch<SetStateAction<ArchiveWorkspaceDialogState | null>>;
  setBatchDeleteDialog: Dispatch<SetStateAction<BatchDeleteDialogState | null>>;
  setDeleteDialog: Dispatch<SetStateAction<DeleteDialogState | null>>;
  setRenameDialog: Dispatch<SetStateAction<RenameDialogState | null>>;
}

export function AppShellDialogs({
  archiveDialog,
  archiveWorkspaceDialog,
  batchDeleteDialog,
  deleteDialog,
  onConfirmArchiveThread,
  onConfirmArchiveWorkspace,
  onConfirmBatchDeleteThread,
  onConfirmDeleteThread,
  onConfirmRenameThread,
  onGenerateTitle,
  renameDialog,
  setArchiveDialog,
  setArchiveWorkspaceDialog,
  setBatchDeleteDialog,
  setDeleteDialog,
  setRenameDialog,
}: AppShellDialogsProps) {
  const { t } = useTranslation("layout");
  return (
    <>
      <ConfirmDialog
        confirmLabel={archiveDialog && archiveDialog.running.length > 0
          ? t("threadDialogs.archiveStopAndArchive")
          : undefined}
        description={archiveDialog ? archiveDescription(archiveDialog, t) : undefined}
        error={archiveDialog?.error ?? null}
        onClose={() => setArchiveDialog(null)}
        onConfirm={onConfirmArchiveThread}
        open={Boolean(archiveDialog)}
        submitting={archiveDialog?.submitting ?? false}
        title={archiveDialog && archiveDialog.running.length > 0
          ? t("threadDialogs.archiveRunningTitle")
          : t("threadDialogs.archiveTitle")}
      >
        {archiveDialog
          ? (
              <div className="space-y-2">
                <div className="rounded-md border border-line-soft bg-surface-subtle p-3 text-sm text-ink">
                  {archiveDialog.thread.title}
                </div>
                {archiveDialog.running.length > 0
                  ? <RunningList count={archiveDialog.running.length} threads={archiveDialog.running} />
                  : null}
              </div>
            )
          : null}
      </ConfirmDialog>
      <ConfirmDialog
        confirmLabel={t("threadDialogs.archiveStopAndArchive")}
        description={archiveWorkspaceDialog
          ? t("threadDialogs.archiveWorkspaceRunningDescription", {
              count: archiveWorkspaceDialog.running.length,
              name: archiveWorkspaceDialog.workspace.name,
            })
          : undefined}
        error={archiveWorkspaceDialog?.error ?? null}
        onClose={() => setArchiveWorkspaceDialog(null)}
        onConfirm={onConfirmArchiveWorkspace}
        open={Boolean(archiveWorkspaceDialog)}
        submitting={archiveWorkspaceDialog?.submitting ?? false}
        title={t("threadDialogs.archiveWorkspaceRunningTitle")}
      >
        {archiveWorkspaceDialog
          ? (
              <div className="space-y-2">
                <div className="rounded-md border border-line-soft bg-surface-subtle p-3 text-sm text-ink">
                  {archiveWorkspaceDialog.workspace.name}
                </div>
                <RunningList count={archiveWorkspaceDialog.running.length} threads={archiveWorkspaceDialog.running} />
              </div>
            )
          : null}
      </ConfirmDialog>
      <RenameDialog
        description={t("appShellDialogs.renameDescription")}
        error={renameDialog?.error ?? null}
        label={t("appShellDialogs.nameLabel")}
        onChange={value =>
          setRenameDialog(current => current ? { ...current, error: null, value } : current)}
        onClose={() => setRenameDialog(null)}
        onConfirm={onConfirmRenameThread}
        onGenerate={onGenerateTitle}
        generating={renameDialog?.generating ?? false}
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

function ConfirmDialog({
  children,
  confirmLabel,
  description,
  error,
  onClose,
  onConfirm,
  open,
  submitting,
  title,
}: {
  children?: React.ReactNode;
  /** What the primary button promises; the archive's own word by default. */
  confirmLabel?: string;
  description?: string;
  error: string | null;
  onClose: () => void;
  onConfirm: () => void;
  open: boolean;
  submitting: boolean;
  title: string;
}) {
  const { t } = useTranslation("layout");
  return (
    <Dialog
      description={description}
      footer={(
        <>
          <Button disabled={submitting} onClick={onClose} type="button" variant="ghost">
            {t("common:cancel")}
          </Button>
          <Button disabled={submitting} onClick={onConfirm} type="button" variant="primary">
            {confirmLabel ?? t("activityRail.archive")}
          </Button>
        </>
      )}
      onClose={onClose}
      open={open}
      title={title}
    >
      {children}
      {error ? <div className="mt-2 text-xs leading-5 text-danger">{error}</div> : null}
    </Dialog>
  );
}

/**
 * The running conversations an archive is waiting on.
 *
 * Named rather than counted alone: a workspace can hold several, and "3 个会话
 * 正在运行" without saying which leaves the reader guessing which stop button
 * to find. Three are named and the rest counted, the same shape the backend's
 * refusal uses.
 */
function RunningList({ count, threads }: { count: number; threads: StoredThread[] }) {
  const { t } = useTranslation("layout");
  const named = threads.slice(0, 3).map(thread => thread.title);
  const more = count - named.length;
  return (
    <ul className="space-y-1 rounded-md border border-line-soft bg-surface p-3 text-xs text-ink-soft" data-testid="archive-running-list">
      {named.map((title, index) => (
        <li key={threads[index]!.id} className="truncate text-ink" title={title}>
          {title}
        </li>
      ))}
      {more > 0
        ? (
            <li className="text-ink-muted">
              {t("threadDialogs.archiveRunningMore", { count: more })}
            </li>
          )
        : null}
    </ul>
  );
}

/**
 * What the archive dialog says.
 *
 * Two facts can be true at once — a fork tree (so more than one conversation
 * goes) and a run still in flight (so the archive has to stop first) — and the
 * reader is asked for the second, so both sentences are drawn rather than one
 * replacing the other.
 */
function archiveDescription(
  state: ArchiveDialogState,
  t: (key: string, options?: Record<string, unknown>) => string,
) {
  const parts: string[] = [];
  if (state.members > 1)
    parts.push(t("threadDialogs.archiveDescription", { count: state.members }));
  if (state.running.length > 0) {
    parts.push(t("threadDialogs.archiveRunningDescription", {
      count: state.running.length,
    }));
  }
  return parts.join(" ");
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
