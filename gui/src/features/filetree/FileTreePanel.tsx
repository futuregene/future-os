import type { MouseEvent as ReactMouseEvent } from "react";
import type { DirEntry } from "../../integrations/storage/files";
import type { LinkMenuItem } from "../markdown/renderers/LinkContextMenu";
import { FolderOpen, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { openPath } from "../../integrations/storage/files";
import { emitFutureEvent } from "../../lib/futureEvents";
import { relativizeWorkspacePath } from "../../lib/workspacePath";
import { FilePreviewOverlay } from "../filepreview/FilePreviewOverlay";
import { previewKindForPath } from "../filepreview/previewKind";
import { LinkContextMenu } from "../markdown/renderers/LinkContextMenu";
import { useLinkContextMenu } from "../markdown/renderers/useLinkContextMenu";
import { FileTreeNode } from "./FileTreeNode";
import { useFileTree } from "./useFileTree";

/**
 * Right-panel "Files" tab: a lazy-loading tree of the active workspace's
 * directory. Directories expand in place (state persists across refreshes and
 * tab switches via {@link useFileTree}); files left-click into an in-app
 * preview when previewable (image / markdown) and otherwise open with the OS
 * default handler — mirroring `FileLink`'s activation.
 *
 * Chat vs workspace differ: a chat's root is a private per-thread scratch dir,
 * so there's no "open workspace" affordance and hidden files stay collapsed by
 * default; a real workspace shows the button and reveals dotfiles by default
 * (users work with `.gitignore`, `.env`, etc. there).
 */
export function FileTreePanel({ rootPath, isWorkspace }: { rootPath: string | null; isWorkspace: boolean }) {
  const { t } = useTranslation("filetree");
  const [showHidden, setShowHidden] = useState(isWorkspace);
  // Re-apply the mode's default when the thread (root) changes — a manual toggle
  // only sticks within the same thread.
  useEffect(() => {
    setShowHidden(isWorkspace);
  }, [rootPath, isWorkspace]);
  const tree = useFileTree(rootPath, showHidden);
  const menu = useLinkContextMenu();
  const [refreshing, setRefreshing] = useState(false);
  const [previewTarget, setPreviewTarget] = useState<DirEntry | null>(null);
  const [menuTarget, setMenuTarget] = useState<DirEntry | null>(null);

  const openWorkspace = useCallback(() => {
    if (rootPath)
      void openPath(rootPath).catch(() => {});
  }, [rootPath]);

  const openEntry = useCallback(async (entry: DirEntry) => {
    try {
      await openPath(entry.path);
    }
    catch {
      emitFutureEvent("toast", { message: t("openFailed"), tone: "error" });
    }
  }, [t]);

  const handleOpenFile = useCallback((entry: DirEntry) => {
    if (entry.isDir)
      return;
    if (previewKindForPath(entry.path)) {
      setPreviewTarget(entry);
    }
    else {
      void openEntry(entry);
    }
  }, [openEntry]);

  // Attach a file as an `@`-mention pill in the active composer. The pill wants
  // a workspace-relative, POSIX-separated path (the same form the mention picker
  // produces); the tree holds absolute paths, so relativize against the root.
  const attachEntry = useCallback((entry: DirEntry) => {
    const relative = relativizeWorkspacePath(entry.path, rootPath).replace(/\\/g, "/");
    emitFutureEvent("attach-file-to-context", { path: relative, name: entry.name });
  }, [rootPath]);

  const handleContextMenu = useCallback((entry: DirEntry, event: ReactMouseEvent<HTMLElement>) => {
    setMenuTarget(entry);
    menu.open(event);
  }, [menu]);

  const menuItems: LinkMenuItem[] = menuTarget
    ? menuTarget.isDir
      ? [{ label: t("menu.open"), onSelect: () => void openEntry(menuTarget) }]
      : [
          { label: t("menu.attach"), onSelect: () => attachEntry(menuTarget) },
          ...(previewKindForPath(menuTarget.path)
            ? [{ label: t("menu.preview"), onSelect: () => setPreviewTarget(menuTarget) }]
            : []),
          { divider: true, label: t("menu.open"), onSelect: () => void openEntry(menuTarget) },
        ]
    : [];

  const handleRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await tree.refresh();
    }
    finally {
      setRefreshing(false);
    }
  }, [tree]);

  const rootEntries = tree.rootEntries;
  const previewKind = previewTarget ? previewKindForPath(previewTarget.path) : null;
  // While the shared cursor-anchored menu is open, keep its target row's `...`
  // trigger visible instead of fading out with hover.
  const activeMenuPath = menu.position ? menuTarget?.path ?? null : null;

  // Anti-flicker (mirrors ContextPanel's delayed spinner): while the root has no
  // entries yet, hold the "loading" line back for a beat. A cache-warm switch
  // back to this tab has entries on the first render and never trips this; only
  // a genuine cold load that outlasts the delay shows the indicator.
  const loadingPending = rootEntries === null && !tree.rootErrored;
  const [showLoading, setShowLoading] = useState(false);
  useEffect(() => {
    if (!loadingPending) {
      setShowLoading(false);
      return;
    }
    const timer = setTimeout(setShowLoading, 200, true);
    return () => clearTimeout(timer);
  }, [loadingPending]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <label className="mr-auto flex cursor-pointer items-center gap-1.5 text-xs text-ink-soft">
          <input
            checked={showHidden}
            className="size-3.5 accent-accent"
            onChange={event => setShowHidden(event.target.checked)}
            type="checkbox"
          />
          {t("showHidden")}
        </label>
        {isWorkspace
          ? (
              <Button
                disabled={!rootPath}
                leftIcon={<FolderOpen className="size-3.5" />}
                onClick={openWorkspace}
                size="sm"
                variant="toolbar"
              >
                {t("openWorkspace")}
              </Button>
            )
          : null}
        <Button
          disabled={refreshing || !rootPath}
          leftIcon={<RefreshCw className={`size-3.5${refreshing ? " animate-spin" : ""}`} />}
          onClick={() => void handleRefresh()}
          size="sm"
          variant="toolbar"
        >
          {refreshing ? t("refreshing") : t("refresh")}
        </Button>
      </div>

      {/* The white list box is always mounted so it never flickers in/out — every
          state (loading, error, empty, tree) renders inside it, centered when
          there's no list. */}
      <div className="min-h-0 flex-1 overflow-auto rounded-md border border-line-soft bg-surface p-1">
        {rootEntries === null
          ? tree.rootErrored
            ? <div className="flex h-full items-center justify-center text-sm text-ink-muted">{t("loadFailed")}</div>
            : showLoading
              ? <div className="flex h-full items-center justify-center text-sm text-ink-muted">{t("loading")}</div>
              : null
          : rootEntries.length === 0
            ? <div className="flex h-full items-center justify-center text-sm text-ink-muted">{t("empty")}</div>
            : (
                <ul>
                  {rootEntries.map(entry => (
                    <FileTreeNode
                      activePath={activeMenuPath}
                      depth={0}
                      entry={entry}
                      key={entry.path}
                      onContextMenu={handleContextMenu}
                      onOpenFile={handleOpenFile}
                      tree={tree}
                    />
                  ))}
                </ul>
              )}
      </div>

      <LinkContextMenu controller={menu} items={menuItems} />

      {previewTarget && previewKind
        ? (
            <FilePreviewOverlay
              kind={previewKind}
              name={previewTarget.name}
              onClose={() => setPreviewTarget(null)}
              onOpenExternal={() => void openEntry(previewTarget)}
              open
              path={previewTarget.path}
            />
          )
        : null}
    </div>
  );
}
