import { MoreHorizontal, Pin, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useDropUpMenu } from "../../components/layout/hooks/useDropUpMenu";
import { MenuPanel } from "../../components/ui/MenuPanel";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { deleteRemoteWorkspace, setRemoteWorkspacePinned } from "./remotePeerClient";

/**
 * The actions a workspace on another machine supports.
 *
 * The same menu the local heading has, reduced to what the host accepts: pin and
 * remove. "Reveal in file manager" is absent because the folder is on *that*
 * machine — there is nothing here to open — and an item that did nothing when
 * clicked would be worse than its absence.
 *
 * Like the remote conversation menu, an item does not dismiss before running: a
 * host command can fail on the wire, and a message that vanished with the menu
 * that produced it is no message at all.
 */
export function RemoteWorkspaceMenu({
  desktopId,
  name,
  onChanged,
  workspaceId,
}: {
  desktopId: string;
  name: string;
  /** Re-read the host's catalogue once the change has landed. */
  onChanged: () => void;
  workspaceId: string;
}) {
  const { t } = useTranslation("layout");
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { menuRef, dropUp } = useDropUpMenu();
  const layerRef = useDismissableLayer<HTMLDivElement>({
    enabled: open,
    onDismiss: () => setOpen(false),
  });

  async function run(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    try {
      await action();
      onChanged();
      setOpen(false);
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(false);
    }
  }

  return (
    <div className="relative" ref={layerRef}>
      <button
        aria-expanded={open}
        aria-haspopup="menu"
        aria-label={t("activityRail.workspaceActions", { name })}
        className={cn(
          // Kept in the layout while hidden, like every other heading control,
          // so revealing it on hover does not shift the heading.
          "inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft",
          open ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
        )}
        onClick={() => setOpen(value => !value)}
        title={t("activityRail.workspaceActions", { name })}
        type="button"
      >
        <MoreHorizontal className="size-3.5" />
      </button>
      {open
        ? (
            <MenuPanel
              className={cn("absolute right-1 z-40 w-40 p-1", dropUp ? "bottom-7" : "top-7")}
              ref={menuRef}
              role="menu"
            >
              {error ? <p className="px-2 py-1 text-xs text-danger">{error}</p> : null}
              <Item disabled={busy} icon={<Pin className="size-3.5" />} onClick={() => void run(() => setRemoteWorkspacePinned(desktopId, workspaceId, true))}>
                {t("activityRail.pin")}
              </Item>
              <Item disabled={busy} icon={<Pin className="size-3.5" />} onClick={() => void run(() => setRemoteWorkspacePinned(desktopId, workspaceId, false))}>
                {t("activityRail.unpin")}
              </Item>
              <Item danger disabled={busy} icon={<Trash2 className="size-3.5" />} onClick={() => void run(() => deleteRemoteWorkspace(desktopId, workspaceId))}>
                {t("activityRail.delete")}
              </Item>
            </MenuPanel>
          )
        : null}
    </div>
  );
}

/** One row of the menu. */
function Item({
  children,
  danger,
  disabled,
  icon,
  onClick,
}: {
  children: string;
  danger?: boolean;
  disabled?: boolean;
  icon: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      className={cn(
        "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm font-medium transition-colors",
        danger ? "text-danger hover:bg-danger-soft" : "text-ink-soft hover:bg-surface-subtle hover:text-ink",
        disabled && "cursor-not-allowed opacity-50",
      )}
      disabled={disabled}
      onClick={onClick}
      role="menuitem"
      type="button"
    >
      {icon}
      <span className="whitespace-nowrap">{children}</span>
    </button>
  );
}
