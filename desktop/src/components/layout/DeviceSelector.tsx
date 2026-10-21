import type { DeviceFilter } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { Check, ChevronDown, Monitor, MonitorSmartphone } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { iconGlyph, peerBadgeText } from "../../features/remote-peer/peerIcons";
import { peerHeld, peerHolderName, peerStatus, peerStatusKey } from "../../features/remote-peer/peerStatus";
import { cn } from "../../lib/cn";
import { useKeyboardOwner } from "../../lib/keyboardOwners";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { MenuPanel } from "../ui/MenuPanel";

/**
 * The device picker above the conversation list.
 *
 * Two states the user needs to tell apart at a glance: "all machines" (a merged
 * list) and "one machine" (that machine's list). The button label says which,
 * because the list below it is otherwise identical-looking either way.
 */
export function DeviceSelector({
  compact = false,
  filter,
  onChange,
  onManage,
  peers,
}: {
  /**
   * Draw the control as one item of a row rather than as a band of its own.
   *
   * The rail's list tools (search, this picker, the list's menu) are one row, so
   * the picker has to give way: it keeps its glyph, its name and its chevron, and
   * the name truncates instead of taking the full width. Its own row — where it
   * is the only thing in it — stays the default.
   */
  compact?: boolean;
  filter: DeviceFilter;
  onChange: (filter: DeviceFilter) => void;
  onManage: () => void;
  peers: RemotePeer[];
}) {
  const { t } = useTranslation("remotePeer");
  const [open, setOpen] = useState(false);
  const menuRef = useDismissableLayer<HTMLDivElement>({
    enabled: open,
    onDismiss: () => setOpen(false),
  });
  // The open picker owns the arrows, not the transcript behind it.
  useKeyboardOwner(open);

  const active = filter.kind === "device"
    ? peers.find(peer => peer.desktopId === filter.desktopId) ?? null
    : null;
  const label = filter.kind === "all"
    ? t("selector.all")
    : active
      ? peerBadgeText(active, active.desktopId)
      : t("selector.local");
  function choose(next: DeviceFilter) {
    onChange(next);
    setOpen(false);
  }

  return (
    <div className={cn("relative", compact ? "shrink-0" : "px-2")}>
      {/* Named explicitly because the visible name starts with a decorative
          glyph: without this the control's accessible name is the emoji, so
          neither a screen reader nor a test can address it as "all devices". */}
      <button
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={label}
        className={cn(
          "flex items-center gap-1.5 rounded-md border border-line-soft px-1.5 text-xs font-medium text-ink transition-colors hover:bg-surface-subtle",
          compact ? "h-7 max-w-30" : "h-8 w-full gap-2 px-2",
        )}
        onClick={() => setOpen(current => !current)}
        type="button"
      >
        <span aria-hidden className="shrink-0 text-sm">
          {filterIcon(filter, peers)}
        </span>
        <span className="min-w-0 flex-1 truncate text-left">{label}</span>
        <ChevronDown className="size-3.5 shrink-0 text-ink-soft" />
      </button>

      {open
        ? (
            <MenuPanel
              className={cn(
                "absolute z-40 p-1",
                compact ? "left-0 top-8 w-56" : "left-2 right-2 top-9",
              )}
              ref={menuRef}
              role="listbox"
            >
              <SelectorOption
                active={filter.kind === "all"}
                label={t("selector.all")}
                onClick={() => choose({ kind: "all" })}
              >
                {filterIcon({ kind: "all" }, peers)}
              </SelectorOption>
              <SelectorOption
                active={filter.kind === "device" && filter.desktopId === null}
                label={t("selector.local")}
                onClick={() => choose({ kind: "device", desktopId: null })}
              >
                {filterIcon({ kind: "device", desktopId: null }, peers)}
              </SelectorOption>
              {peers.map(peer => (
                <SelectorOption
                  active={filter.kind === "device" && filter.desktopId === peer.desktopId}
                  key={peer.desktopId}
                  label={peerBadgeText(peer, peer.desktopId)}
                  // A disconnected machine stays selectable: its rows are gone
                  // anyway, and disabling the entry would leave no way to see
                  // that it is merely offline rather than removed. The note says
                  // *which* kind of not-there it is — being reached, unreachable,
                  // or switched off by the user, whose own row offers the way
                  // back. A connected machine that another paired device drives
                  // gets the same treatment for the same reason: it is here,
                  // answering, and its rows are missing.
                  note={peerNote(peer, t)}
                  onClick={() => choose({ kind: "device", desktopId: peer.desktopId })}
                >
                  {filterIcon({ kind: "device", desktopId: peer.desktopId }, peers)}
                </SelectorOption>
              ))}
              <div className="my-1 h-px bg-line-soft" />
              <button
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs text-accent hover:bg-surface-subtle"
                onClick={() => {
                  setOpen(false);
                  onManage();
                }}
                type="button"
              >
                {t("selector.manage")}
              </button>
            </MenuPanel>
          )
        : null}
    </div>
  );
}

/**
 * The glyph that stands for a filter.
 *
 * One function for both the collapsed button and the expanded entries, because
 * they showed different pictures for the same choice (🗂 for "all devices"
 * collapsed, a monitor for the same entry expanded) — the button has to show
 * the entry the user picked, or it looks like the selection did not take.
 */
function filterIcon(filter: DeviceFilter, peers: RemotePeer[]): React.ReactNode {
  if (filter.kind === "all")
    return <MonitorSmartphone className="size-3.5" />;
  if (filter.desktopId === null)
    return <Monitor className="size-3.5" />;
  return <span aria-hidden>{iconGlyph(peers.find(peer => peer.desktopId === filter.desktopId)?.icon)}</span>;
}

/**
 * What an entry says under its name when the machine is not simply usable.
 *
 * Three answers, and they are three different situations: the machine is not
 * there (offline / being reached / switched off), it answers while another
 * paired device drives it, or nothing is wrong and it says nothing. The held
 * case names the holder when it declared a name — the same sentence the phone's
 * sheet uses, because it is the same fact — and stays neutral when it did not.
 */
function peerNote(
  peer: RemotePeer,
  t: (key: string, options?: Record<string, unknown>) => string,
): string | undefined {
  if (peerHeld(peer)) {
    const name = peerHolderName(peer);
    return name ? t("heldBy", { name }) : t("heldByUnknown");
  }
  return peer.connected ? undefined : t(peerStatusKey(peerStatus(peer)));
}
function SelectorOption({
  active,
  children,
  label,
  note,
  onClick,
}: {
  active: boolean;
  children: React.ReactNode;
  label: string;
  note?: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      aria-selected={active}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs",
        active ? "bg-accent-soft font-medium text-accent" : "text-ink hover:bg-surface-subtle",
      )}
      onClick={onClick}
      role="option"
      type="button"
    >
      <span className="shrink-0">{children}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {note ? <span className="shrink-0 text-3xs text-ink-muted" data-testid="option-note">{note}</span> : null}
      {active ? <Check className="size-3.5 shrink-0" /> : null}
    </button>
  );
}
