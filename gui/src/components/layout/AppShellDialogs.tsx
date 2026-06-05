import type { Dispatch, SetStateAction } from "react";
import type { StoredThread, ThreadCleanupSummary } from "../../integrations/storage/threadStore";
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
  return (
    <>
      <Dialog
        description="Give this conversation a short name that will be easy to find in the sidebar."
        footer={(
          <>
            <Button
              disabled={renameDialog?.submitting}
              onClick={() => setRenameDialog(null)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              disabled={renameDialog?.submitting}
              onClick={() => onConfirmRenameThread()}
              type="button"
              variant="primary"
            >
              {renameDialog?.submitting ? "Saving..." : "Save"}
            </Button>
          </>
        )}
        onClose={() => setRenameDialog(null)}
        open={Boolean(renameDialog)}
        title="Rename Chat"
      >
        <label className="block text-sm font-medium text-ink-soft" htmlFor="thread-title">
          Name
        </label>
        <input
          autoFocus
          className="mt-2 h-10 w-full rounded-md border border-line bg-white px-3 text-sm text-ink outline-none transition focus:border-accent focus:ring-2 focus:ring-accent/15"
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
        {renameDialog?.error ? <div className="mt-2 text-xs leading-5 text-red-600">{renameDialog.error}</div> : null}
      </Dialog>
      <Dialog
        description={deleteDialog ? deleteThreadDescription(deleteDialog.thread) : undefined}
        footer={(
          <>
            <Button
              disabled={deleteDialog?.submitting}
              onClick={() => setDeleteDialog(null)}
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
            <Button
              disabled={deleteDialog?.submitting}
              onClick={() => onConfirmDeleteThread()}
              type="button"
              variant="danger"
            >
              {deleteDialog?.submitting ? "Deleting..." : "Delete"}
            </Button>
          </>
        )}
        onClose={() => setDeleteDialog(null)}
        open={Boolean(deleteDialog)}
        title="Delete Chat"
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
        {deleteDialog?.error ? <div className="mt-2 text-xs leading-5 text-red-600">{deleteDialog.error}</div> : null}
      </Dialog>
    </>
  );
}

function ArtifactCount({ count }: { count: number }) {
  return (
    <div className="flex items-center justify-between rounded-md border border-line-soft bg-white px-3 py-2 text-sm">
      <span className="text-ink-soft">Artifacts</span>
      <span className="font-semibold text-ink">{count}</span>
    </div>
  );
}

function deleteThreadDescription(thread: StoredThread) {
  if (thread.mode === "workspace") {
    return "This removes only the chat. Workspace files will not be changed.";
  }

  return "This chat will be removed from the sidebar.";
}
