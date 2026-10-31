import type { MouseEvent as ReactMouseEvent } from "react";
import type { DirEntry } from "../../integrations/storage/files";
import type { LinkMenuItem } from "../markdown/renderers/LinkContextMenu";
import { FolderOpen, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { FloatingScrollbar } from "../../components/ui/FloatingScrollbar";
import { openPath } from "../../integrations/storage/files";
import { emitFutureEvent, onFutureEvent } from "../../lib/futureEvents";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { relativizeWorkspacePath } from "../../lib/workspacePath";
import { FilePreviewOverlay } from "../filepreview/FilePreviewOverlay";
import { previewKindForPath } from "../filepreview/previewKind";
import { LinkContextMenu } from "../markdown/renderers/LinkContextMenu";
import { useLinkContextMenu } from "../markdown/renderers/useLinkContextMenu";
import { FileTreeNode } from "./FileTreeNode";
import { parentDirectory, siblingImages } from "./imageSiblings";
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
export function FileTreePanel({
  rootPath,
  isWorkspace,
}: {
  rootPath: string | null;
  isWorkspace: boolean;
}) {
  const { t } = useTranslation("filetree");
  const [showHidden, setShowHidden] = useState(isWorkspace);
  // Re-apply the mode's default when the thread (root) changes — a manual toggle
  // only sticks within the same thread.
  useEffect(() => {
    setShowHidden(isWorkspace);
  }, [rootPath, isWorkspace]);
  const tree = useFileTree(rootPath, showHidden);
  const menu = useLinkContextMenu();
  const listScrollbar = useFloatingScrollbar();
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Auto-refresh on tool lifecycle/termination so newly created or modified
  // files appear without manual intervention. Coalesce bursts to at most one
  // refresh per 2s; text/thinking-only stream updates do not invalidate the tree.
  const { refresh: refreshTree } = tree;
  useEffect(() => {
    const unsubscribe = onFutureEvent("file-tree-refresh", () => {
      if (refreshTimerRef.current)
        return; // already scheduled
      refreshTimerRef.current = setTimeout(() => {
        refreshTimerRef.current = null;
        void refreshTree();
      }, 2000);
    });
    return () => {
      unsubscribe();
      if (refreshTimerRef.current) {
        clearTimeout(refreshTimerRef.current);
        refreshTimerRef.current = null;
      }
    };
  }, [refreshTree]);
  const [refreshing, setRefreshing] = useState(false);
  const [previewTarget, setPreviewTarget] = useState<DirEntry | null>(null);
  const [menuTarget, setMenuTarget] = useState<DirEntry | null>(null);

  const openWorkspace = useCallback(() => {
    if (rootPath)
      void openPath(rootPath).catch(() => {});
  }, [rootPath]);

  const openEntry = useCallback(
    async (entry: DirEntry) => {
      try {
        await openPath(entry.path);
      }
      catch {
        emitFutureEvent("toast", { message: t("openFailed"), tone: "error" });
      }
    },
    [t],
  );

  const handleOpenFile = useCallback(
    (entry: DirEntry) => {
      if (entry.isDir)
        return;
      if (previewKindForPath(entry.path)) {
        setPreviewTarget(entry);
      }
      else {
        void openEntry(entry);
      }
    },
    [openEntry],
  );

  // Attach a file as an `@`-mention pill in the active composer. The pill wants
  // a workspace-relative, POSIX-separated path (the same form the mention picker
  // produces); the tree holds absolute paths, so relativize against the root.
  const attachEntry = useCallback(
    (entry: DirEntry) => {
      const relative = relativizeWorkspacePath(entry.path, rootPath).replace(
        /\\/g,
        "/",
      );
      emitFutureEvent("attach-file-to-context", {
        path: relative,
        name: entry.name,
      });
    },
    [rootPath],
  );

  const handleContextMenu = useCallback(
    (entry: DirEntry, event: ReactMouseEvent<HTMLElement>) => {
      setMenuTarget(entry);
      menu.open(event);
    },
    [menu],
  );

  const menuItems: LinkMenuItem[] = menuTarget
    ? menuTarget.isDir
      ? [{ label: t("menu.open"), onSelect: () => void openEntry(menuTarget) }]
      : [
          { label: t("menu.attach"), onSelect: () => attachEntry(menuTarget) },
          ...(previewKindForPath(menuTarget.path)
            ? [
                {
                  label: t("menu.preview"),
                  onSelect: () => setPreviewTarget(menuTarget),
                },
              ]
            : []),
          {
            divider: true,
            label: t("menu.open"),
            onSelect: () => void openEntry(menuTarget),
          },
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
  const previewKind = previewTarget
    ? previewKindForPath(previewTarget.path)
    : null;

  // Pictures step through their own folder: the ← / → controls walk the images
  // of the directory the row was opened from, in the order the tree lists them
  // (so "next" is the row below). The listing is already in hand — the row could
  // not have been drawn otherwise — so this costs no extra read.
  const previewDirectory = previewTarget ? parentDirectory(previewTarget.path) : "";
  const previewSiblings = previewTarget && previewKind === "image"
    ? siblingImages(
        previewDirectory === (rootPath ?? "").replace(/[/\\]+$/, "")
          ? tree.rootEntries
          : tree.childrenOf(previewDirectory),
        previewTarget.path,
      )
    : [];
  const previewIndex = previewTarget
    ? previewSiblings.findIndex(entry => entry.path === previewTarget.path)
    : -1;
  // A lone image has nothing to step to: no list, no controls.
  const previewNavigation = previewIndex >= 0 && previewSiblings.length > 1
    ? {
        count: previewSiblings.length,
        index: previewIndex,
        onNext: () => {
          const next = previewSiblings[previewIndex + 1];
          if (next)
            setPreviewTarget(next);
        },
        onPrev: () => {
          const prev = previewSiblings[previewIndex - 1];
          if (prev)
            setPreviewTarget(prev);
        },
      }
    : undefined;
  // While the shared cursor-anchored menu is open, keep its target row's `...`
  // trigger visible instead of fading out with hover.
  const activeMenuPath = menu.position ? (menuTarget?.path ?? null) : null;

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
      <div className="flex shrink-0 items-center justify-between gap-2 px-4 py-1.5">
        <label className="flex min-w-0 cursor-pointer items-center gap-1.5 text-xs text-ink-soft">
          <input
            checked={showHidden}
            className="size-3.5 accent-accent"
            onChange={event => setShowHidden(event.target.checked)}
            type="checkbox"
          />
          {t("showHidden")}
        </label>
        <div className="flex shrink-0 items-center gap-2">
          <Button
            disabled={!rootPath}
            leftIcon={<FolderOpen className="size-3.5" />}
            onClick={openWorkspace}
            size="sm"
            variant="toolbar"
          >
            {t("openWorkspace")}
          </Button>
          <Button
            disabled={refreshing || !rootPath}
            leftIcon={(
              <RefreshCw
                className={`size-3.5${refreshing ? " animate-spin" : ""}`}
              />
            )}
            onClick={() => void handleRefresh()}
            size="sm"
            variant="toolbar"
          >
            {t("refresh")}
          </Button>
        </div>
      </div>

      {/* Every state stays mounted in one edge-to-edge scroll region so the
          boundary and scrollbar do not jump while the tree loads. */}
      <div className="group relative flex min-h-0 flex-1 flex-col border-t border-line-soft/70">
        <div
          className="floating-scrollbar min-h-0 flex-1 overflow-x-hidden overflow-y-auto bg-surface/50 [&>ul>li:first-child>div]:border-t-0"
          onScroll={listScrollbar.handleScroll}
          ref={listScrollbar.scrollRef}
        >
          {rootEntries === null
            ? (
                tree.rootErrored
                  ? (
                      <div className="flex h-full items-center justify-center text-sm text-ink-muted">
                        {t("loadFailed")}
                      </div>
                    )
                  : showLoading
                    ? (
                        <div className="flex h-full items-center justify-center text-sm text-ink-muted">
                          {t("loading")}
                        </div>
                      )
                    : null
              )
            : rootEntries.length === 0
              ? (
                  <div className="flex h-full items-center justify-center text-sm text-ink-muted">
                    {t("empty")}
                  </div>
                )
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
        <FloatingScrollbar
          onPointerDown={listScrollbar.handleThumbPointerDown}
          scrollbar={listScrollbar.scrollbar}
        />
        {/*
          The panel's closing edge.

          The Files list is the last thing in a full-height column, so without a
          rule of its own a long tree simply ran out at the window's bottom edge —
          the last row sliced through its glyphs, nothing under it, which reads as
          a list spilling out of the panel rather than one that ends. The hairline
          belongs to this strip and not to the scroll viewport: the viewport is
          what scrolls, and a border on it would travel with the content and sit
          on the window edge. Here it stays put, sits below the viewport, and has
          a few pixels of the panel's own surface under it, so it is never the
          last pixel row of the window.
        */}
        <div
          aria-hidden="true"
          className="h-1.5 shrink-0 border-t border-line-soft"
          data-testid="file-tree-bottom-edge"
        />
      </div>

      <LinkContextMenu controller={menu} items={menuItems} />

      {previewTarget && previewKind
        ? (
            <FilePreviewOverlay
              kind={previewKind}
              name={previewTarget.name}
              navigation={previewNavigation}
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
