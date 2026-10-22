import type { RemoteWorkspaceRow } from "./remotePeerClient";
import { Check, Folder, FolderOpen, Loader2, MessageSquare } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { MenuPanel } from "../../components/ui/MenuPanel";
import { cn } from "../../lib/cn";
import { useKeyboardOwner } from "../../lib/keyboardOwners";
import { useDismissableLayer } from "../../lib/useDismissableLayer";

/**
 * The row under a remote draft: which workspace on that host it belongs to, or
 * a plain conversation.
 *
 * This is the local new-conversation screen's own footer, drawn for a machine
 * that is not this one, because a remote draft used to have no such choice at
 * all: it opened on a workspace only when it was started from that workspace's
 * heading, and the "new conversation" path silently made a chat. The reader
 * could not say where the conversation should live, which is the one decision
 * the local screen puts in front of them before the first prompt.
 *
 * What it cannot copy is the local path picker: the folders are on *that* disk.
 * So the menu lists what the host already knows about, and the "open a
 * workspace" item hands over to the host's create form (whose path is typed
 * rather than browsed, for the same reason). A machine with no workspaces yet
 * says so instead of offering an empty list.
 *
 * `workspaceId === null` means "no workspace": a chat conversation. That is the
 * host's own signal when the first prompt creates the session, so the choice is
 * carried rather than translated.
 */
export function RemoteDraftTargetChooser({
  className,
  error,
  loading = false,
  onOpenWorkspace,
  onSelect,
  rows,
  workspaceId,
}: {
  className?: string;
  /**
   * Why the host's workspace list could not be read, if it could not.
   *
   * Shown in the menu instead of "no workspaces": the two are different facts,
   * and an unreachable host must not look like an empty disk.
   */
  error?: string | null;
  /** Whether the list is being (re)read, so the menu can say so. */
  loading?: boolean;
  /**
   * Open an existing directory on that host as a workspace.
   *
   * Absent when this side cannot address the host (its row has not loaded yet),
   * in which case the item is left out rather than shown dead.
   */
  onOpenWorkspace?: () => void;
  /** The chosen target: a workspace id on that host, or `null` for a chat. */
  onSelect: (workspaceId: string | null) => void;
  /** That host's workspaces, as its own store reports them. */
  rows: RemoteWorkspaceRow[];
  /** The workspace this draft is in, or `null` for a chat conversation. */
  workspaceId: string | null;
}) {
  const { t } = useTranslation("remotePeer");
  const [menuOpen, setMenuOpen] = useState(false);
  const menuLayerRef = useDismissableLayer<HTMLDivElement>({
    enabled: menuOpen,
    onDismiss: () => setMenuOpen(false),
  });
  // An open menu owns the arrows; the transcript behind it must not also scroll.
  useKeyboardOwner(menuOpen);

  // The name is the host's label for the folder, and the id is the only thing
  // that means anything to it — a draft in a workspace whose row has not arrived
  // yet shows the generic word rather than an id.
  const selected = workspaceId
    ? (rows.find(row => row.id === workspaceId) ?? null)
    : null;

  return (
    <div
      className={cn(
        // The local footer's own measurements — same height, same radius, same
        // tint, same row of controls — closed with a border because here it is a
        // sibling of the composer box rather than a child of one.
        "relative flex flex-wrap items-center gap-1.5 rounded-b-lg border-x border-b border-line bg-surface-subtle/80 p-1.5",
        className,
      )}
      data-testid="remote-draft-target"
    >
      <div className="relative" ref={menuLayerRef}>
        <button
          aria-expanded={menuOpen}
          aria-haspopup="menu"
          className={cn(
            "inline-flex h-8 max-w-64 items-center gap-2 rounded-md px-2.5 text-sm font-medium transition-colors",
            workspaceId
              ? "bg-surface text-ink shadow-xs"
              : "text-ink-soft hover:bg-surface/70 hover:text-ink",
          )}
          onClick={() => setMenuOpen(open => !open)}
          title={selected?.name ?? t("workspacesSection")}
          type="button"
        >
          <Folder className="size-4 shrink-0" />
          <span className="truncate">{selected?.name ?? t("workspacesSection")}</span>
        </button>
        {menuOpen
          ? (
              <MenuPanel
                // Opens *upward*: the composer rides at the foot of the window,
                // so a menu below it would be off screen.
                className="absolute bottom-full left-0 z-30 mb-1 w-72 p-1.5"
                role="menu"
              >
                <div className="max-h-36 space-y-0.5 overflow-auto">
                  {error
                    ? <div className="p-2 text-sm text-danger">{error}</div>
                    : null}
                  {!error && loading
                    ? (
                        <div className="flex items-center gap-2 p-2 text-sm text-ink-muted">
                          <Loader2 className="size-3.5 animate-spin" />
                          {t("workspacesLoading")}
                        </div>
                      )
                    : null}
                  {!error && !loading && rows.length === 0
                    ? <div className="p-2 text-sm text-ink-muted">{t("workspacesEmpty")}</div>
                    : null}
                  {rows.map(workspace => (
                    <button
                      className="flex h-9 w-full items-center gap-2 rounded-md px-2 text-left transition-colors hover:bg-surface-subtle"
                      key={workspace.id}
                      onClick={() => {
                        onSelect(workspace.id);
                        setMenuOpen(false);
                      }}
                      role="menuitem"
                      type="button"
                    >
                      <Folder className="size-4 shrink-0 text-ink-soft" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium text-ink">{workspace.name}</span>
                        {/* The path is that machine's, which is exactly what the
                            reader needs to tell two similarly named folders
                            apart. */}
                        {workspace.path
                          ? <span className="block truncate text-xs text-ink-muted">{workspace.path}</span>
                          : null}
                      </span>
                      {workspaceId === workspace.id ? <Check className="size-4 text-ink-soft" /> : null}
                    </button>
                  ))}
                </div>
                <div className="mt-1 space-y-0.5 border-t border-line-soft pt-1">
                  {onOpenWorkspace
                    ? (
                        <button
                          className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                          onClick={() => {
                            setMenuOpen(false);
                            onOpenWorkspace();
                          }}
                          role="menuitem"
                          type="button"
                        >
                          <FolderOpen className="size-3.5" />
                          <span>{t("draftOpenWorkspace")}</span>
                        </button>
                      )
                    : null}
                  {/* The same second entry the local menu carries, so either
                      screen lets the reader change their mind about the place
                      the conversation starts in. */}
                  <button
                    className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                    onClick={() => {
                      onSelect(null);
                      setMenuOpen(false);
                    }}
                    role="menuitem"
                    type="button"
                  >
                    <MessageSquare className="size-3.5" />
                    <span>{t("draftChat")}</span>
                  </button>
                </div>
              </MenuPanel>
            )
          : null}
      </div>
      {/* The chat answer is a control of its own, not only a menu item, because
          it is the other half of the question the workspace button asks — the
          two sit side by side on the local screen in exactly this arrangement. */}
      <button
        className={cn(
          "inline-flex h-8 items-center gap-2 rounded-md px-3 text-sm font-medium transition-colors",
          workspaceId
            ? "text-ink-soft hover:bg-surface/70 hover:text-ink"
            : "bg-surface text-ink shadow-xs",
        )}
        onClick={() => {
          onSelect(null);
          setMenuOpen(false);
        }}
        type="button"
      >
        <MessageSquare className="size-4" />
        <span>{t("draftChat")}</span>
      </button>
      <div className="min-w-56 flex-1 px-1 text-xs leading-5 text-ink-muted">
        {workspaceId ? t("draftWorkspaceHint") : t("draftChatHint")}
      </div>
    </div>
  );
}
