import type { MouseEvent as ReactMouseEvent } from "react";
import type { DirEntry } from "../../integrations/storage/files";
import type { FileTree } from "./useFileTree";
import { ChevronDown, ChevronRight, MoreHorizontal } from "lucide-react";
import { useTranslation } from "react-i18next";
import { FileTypeIcon } from "../../components/ui/FileTypeIcon";
import { cn } from "../../lib/cn";
import { fileKind } from "../../lib/fileType";

/** Left indent per tree depth, in px. Depth 0 = root's direct children. */
const INDENT_STEP = 12;
const BASE_INDENT = 4;

export interface FileTreeNodeProps {
  entry: DirEntry;
  depth: number;
  tree: FileTree;
  /**
   * Path of the entry whose actions menu is currently open, if any. The matching
   * row keeps its `...` trigger visible (rather than fading out with hover) while
   * the menu stays open — mirrors the artifacts / thread lists.
   */
  activePath?: string | null;
  /**
   * Open the actions menu for an entry — wired to both the row's right-click and
   * its `...` trigger. The menu is cursor-anchored, so a left-click event works
   * the same as a context-menu event.
   */
  onContextMenu?: (entry: DirEntry, event: ReactMouseEvent<HTMLElement>) => void;
}

/**
 * One row in the file tree, plus its expanded children rendered recursively.
 * Directories toggle on click and lazy-load; files do nothing on left-click —
 * open/preview/attach are reached through the right-click menu. Directories are
 * marked by the expand chevron alone (no folder glyph); files show their
 * {@link FileTypeIcon} in the same chevron-sized slot so names stay aligned.
 */
export function FileTreeNode({ entry, depth, tree, activePath, onContextMenu }: FileTreeNodeProps) {
  const { t } = useTranslation("filetree");
  const expanded = entry.isDir && tree.isExpanded(entry.path);
  const children = tree.childrenOf(entry.path);
  const loading = tree.isLoading(entry.path);
  const errored = tree.isErrored(entry.path);
  const menuActive = activePath === entry.path;

  function handleClick() {
    if (entry.isDir)
      tree.toggle(entry.path);
  }

  return (
    <li>
      {/* Row is a flex container rather than a single button so the `...` trigger
          can sit inside without nesting buttons; the leading part stays the
          click/expand target. */}
      <div
        className="group flex w-full items-center rounded-md pr-1 text-sm text-ink transition-colors hover:bg-surface-subtle"
        style={{ paddingLeft: BASE_INDENT + depth * INDENT_STEP }}
      >
        <button
          className="flex min-w-0 flex-1 items-center gap-1.5 py-1 text-left"
          onClick={handleClick}
          onContextMenu={onContextMenu ? event => onContextMenu(entry, event) : undefined}
          title={entry.name}
          type="button"
        >
          {/* One leading slot, sized to the chevron so file and folder names align:
              directories show the expand chevron (no folder glyph), files show
              their small type icon in the same column. */}
          <span className="flex size-4 shrink-0 items-center justify-center">
            {entry.isDir
              ? expanded
                ? <ChevronDown className="size-3.5 text-ink-muted" />
                : <ChevronRight className="size-3.5 text-ink-muted" />
              : <FileTypeIcon className="size-3.5 text-ink-soft" kind={fileKind(entry.path)} />}
          </span>
          <span className="truncate">{entry.name}</span>
        </button>
        {onContextMenu
          ? (
              <button
                aria-label={t("actions", { name: entry.name })}
                className={cn(
                  "inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted opacity-0 transition-colors hover:bg-surface hover:text-ink-soft group-hover:opacity-100",
                  menuActive && "opacity-100",
                )}
                onClick={(event) => {
                  event.stopPropagation();
                  onContextMenu(entry, event);
                }}
                title={t("actions", { name: entry.name })}
                type="button"
              >
                <MoreHorizontal className="size-3.5" />
              </button>
            )
          : null}
      </div>

      {expanded
        ? (
            <ul>
              {errored
                ? (
                    <li>
                      <button
                        className="flex items-center gap-1 rounded-md py-1 text-xs text-danger hover:underline"
                        onClick={() => tree.reload(entry.path)}
                        style={{ paddingLeft: BASE_INDENT + (depth + 1) * INDENT_STEP }}
                        type="button"
                      >
                        {t("loadFailed")}
                        {" · "}
                        {t("retry")}
                      </button>
                    </li>
                  )
                : children === null
                  ? loading
                    ? <li className="py-1 text-xs text-ink-muted" style={{ paddingLeft: BASE_INDENT + (depth + 1) * INDENT_STEP }}>{t("loading")}</li>
                    : null
                  : children.length === 0
                    ? <li className="py-1 text-xs text-ink-muted" style={{ paddingLeft: BASE_INDENT + (depth + 1) * INDENT_STEP }}>{t("empty")}</li>
                    : children.map(child => (
                        <FileTreeNode
                          activePath={activePath}
                          depth={depth + 1}
                          entry={child}
                          key={child.path}
                          onContextMenu={onContextMenu}
                          tree={tree}
                        />
                      ))}
            </ul>
          )
        : null}
    </li>
  );
}
