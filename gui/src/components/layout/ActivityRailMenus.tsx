import type { ReactNode } from "react";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import { Archive, FolderOpen, MoreHorizontal, Pencil, Pin, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { openPath } from "../../integrations/storage/files";
import { cn } from "../../lib/cn";
import { isMacOS, isWindows } from "../../lib/platform";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { MenuPanel } from "../ui/MenuPanel";
import { useDropUpMenu } from "./hooks/useDropUpMenu";

/** Per-thread actions dropdown (rename / pin / delete, or restore when archived). */
export function ThreadItemMenu({
  archived,
  pinned,
  onClose,
  onDelete,
  onRename,
  onRestore,
  onTogglePin,
}: {
  archived?: boolean;
  pinned: boolean;
  onClose: () => void;
  onDelete: () => void;
  onRename: () => void;
  onRestore: () => void;
  onTogglePin: () => void;
}) {
  const { t } = useTranslation("layout");
  const { menuRef, dropUp } = useDropUpMenu();

  return (
    <MenuPanel
      ref={menuRef}
      className={cn(
        "absolute right-1 z-40 w-36 p-1",
        dropUp ? "bottom-7" : "top-7",
      )}
    >
      {archived
        ? (
            <ThreadMenuItem icon={<Archive className="size-3.5" />} onClick={onRestore} onClose={onClose}>
              {t("activityRail.restore")}
            </ThreadMenuItem>
          )
        : (
            <>
              <ThreadMenuItem icon={<Pencil className="size-3.5" />} onClick={onRename} onClose={onClose}>
                {t("activityRail.rename")}
              </ThreadMenuItem>
              <ThreadMenuItem icon={<Pin className="size-3.5" />} onClick={onTogglePin} onClose={onClose}>
                {pinned ? t("activityRail.unpin") : t("activityRail.pin")}
              </ThreadMenuItem>
            </>
          )}
      <ThreadMenuItem danger icon={<Trash2 className="size-3.5" />} onClick={onDelete} onClose={onClose}>
        {t("activityRail.delete")}
      </ThreadMenuItem>
    </MenuPanel>
  );
}

/**
 * Workspace-header actions dropdown (rename / reveal in file manager / delete).
 * Open state is controlled by the caller so the same menu can be triggered from
 * either the `...` button or a right-click on the workspace row.
 */
export function WorkspaceHeaderMenu({
  workspace,
  open,
  onDelete,
  onOpenChange,
  onRename,
}: {
  workspace: StoredWorkspace;
  open: boolean;
  onDelete: (workspace: StoredWorkspace) => void;
  onOpenChange: (open: boolean) => void;
  onRename: (workspace: StoredWorkspace) => void;
}) {
  const { t } = useTranslation("layout");
  // Label follows OS convention: Finder (macOS) / File Explorer (Windows) /
  // File Manager (Linux and other).
  const revealLabel = isMacOS
    ? t("activityRail.revealInFinder")
    : isWindows
      ? t("activityRail.revealInExplorer")
      : t("activityRail.revealInFileManager");
  const layerRef = useDismissableLayer<HTMLDivElement>({ enabled: open, onDismiss: () => onOpenChange(false) });
  const { menuRef, dropUp } = useDropUpMenu(open);

  return (
    <div className="relative" ref={layerRef}>
      <button
        aria-label={t("activityRail.workspaceActions", { name: workspace.name })}
        className={cn(
          "inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted opacity-0 transition hover:bg-surface hover:text-ink-soft group-hover:opacity-100",
          open && "opacity-100",
        )}
        onClick={(event) => {
          event.stopPropagation();
          onOpenChange(!open);
        }}
        title={t("activityRail.workspaceActions", { name: workspace.name })}
        type="button"
      >
        <MoreHorizontal className="size-3.5" />
      </button>
      {open
        ? (
            <MenuPanel
              ref={menuRef}
              className={cn(
                "absolute right-0 z-40 w-max min-w-36 p-1",
                dropUp ? "bottom-7" : "top-7",
              )}
            >
              <ThreadMenuItem icon={<Pencil className="size-3.5" />} onClick={() => onRename(workspace)} onClose={() => onOpenChange(false)}>
                {t("activityRail.rename")}
              </ThreadMenuItem>
              <ThreadMenuItem icon={<FolderOpen className="size-3.5" />} onClick={() => void openPath(workspace.path).catch(() => {})} onClose={() => onOpenChange(false)}>
                {revealLabel}
              </ThreadMenuItem>
              <ThreadMenuItem danger icon={<Trash2 className="size-3.5" />} onClick={() => onDelete(workspace)} onClose={() => onOpenChange(false)}>
                {t("activityRail.delete")}
              </ThreadMenuItem>
            </MenuPanel>
          )
        : null}
    </div>
  );
}

function ThreadMenuItem({
  children,
  danger,
  icon,
  onClick,
  onClose,
}: {
  children: string;
  danger?: boolean;
  icon: ReactNode;
  onClick: () => void;
  onClose: () => void;
}) {
  return (
    <button
      className={cn(
        "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm font-medium transition-colors",
        danger ? "text-danger hover:bg-danger-soft" : "text-ink-soft hover:bg-surface-subtle hover:text-ink",
      )}
      onClick={() => {
        onClose();
        onClick();
      }}
      type="button"
    >
      {icon}
      <span className="whitespace-nowrap">{children}</span>
    </button>
  );
}
