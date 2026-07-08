import type { FormEvent } from "react";
import { FolderOpen } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { cn } from "../../lib/cn";

/**
 * Modal for the open/create-workspace flow. Its state machine lives in
 * {@link useWorkspaceForm}; this component is purely presentational.
 */
export function WorkspaceModal({
  creating,
  displayName,
  error,
  notice,
  path,
  onCancel,
  onDisplayNameChange,
  onPickFolder,
  onSubmit,
}: {
  creating: boolean;
  displayName: string;
  error: string | null;
  notice: string | null;
  path: string;
  onCancel: () => void;
  onDisplayNameChange: (value: string) => void;
  onPickFolder: () => void;
  onSubmit: (event: FormEvent) => void;
}) {
  const { t } = useTranslation("agent");
  const pathPlaceholder = t("newConversation.modal.selectExisting");

  return (
    <div className="absolute inset-0 z-40 flex items-center justify-center bg-ink-strong/20 px-6">
      <form className="w-full max-w-md rounded-lg border border-line-soft bg-surface p-4 shadow-panel" onSubmit={onSubmit}>
        <div className="mb-3">
          <div className="text-sm font-semibold text-ink">{t("newConversation.modal.openTitle")}</div>
          <div className="mt-1 text-xs leading-5 text-ink-muted">{t("newConversation.modal.openDescription")}</div>
        </div>
        <div className="space-y-2">
          <button
            aria-label={t("newConversation.modal.chooseWorkspaceAria")}
            className="flex h-9 w-full overflow-hidden rounded-md border border-line-soft bg-surface text-left transition-colors hover:bg-surface-subtle"
            onClick={onPickFolder}
            title={t("newConversation.modal.chooseWorkspaceAria")}
            type="button"
          >
            <span className="inline-flex h-full w-10 shrink-0 items-center justify-center border-r border-line-soft text-ink-soft">
              <FolderOpen className="size-4" />
            </span>
            <span
              className={cn(
                "flex min-w-0 flex-1 items-center px-2 text-sm",
                path ? "text-ink" : "text-ink-muted",
              )}
            >
              <span className="truncate">{path || pathPlaceholder}</span>
            </span>
          </button>
          <input
            className="h-9 w-full rounded-md border border-line-soft bg-surface px-2 text-sm text-ink outline-none focus:border-accent"
            onChange={event => onDisplayNameChange(event.target.value)}
            placeholder={t("newConversation.modal.displayNameOpen")}
            value={displayName}
          />
          <div className="min-h-5 truncate px-1 text-xs text-ink-muted" title={path || undefined}>
            {path
              ? t("newConversation.modal.pathPreview", { path })
              : t("newConversation.modal.chooseExisting")}
          </div>
        </div>
        {error ? <div className="mt-2 text-xs leading-5 text-danger">{error}</div> : null}
        {!error && notice ? <div className="mt-2 text-xs leading-5 text-warning">{notice}</div> : null}
        <div className="mt-4 flex justify-end gap-2">
          <button
            className="h-8 rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
            onClick={onCancel}
            type="button"
          >
            {t("newConversation.modal.cancel")}
          </button>
          <Button
            disabled={creating}
            size="sm"
            type="submit"
            variant="primary"
          >
            {t("newConversation.modal.open")}
          </Button>
        </div>
      </form>
    </div>
  );
}
