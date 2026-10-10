import type { DeviceFilter } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { Check, ChevronDown, Monitor, MonitorSmartphone } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { iconGlyph, peerBadgeText } from "../../features/remote-peer/peerIcons";
import { cn } from "../../lib/cn";
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
  filter,
  onChange,
  onManage,
  peers,
}: {
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
    <div className="relative px-2">
      <button
        aria-expanded={open}
        aria-haspopup="listbox"
        className={cn(
          "flex h-8 w-full items-center gap-2 rounded-md border border-line-soft px-2 text-xs font-medium text-ink transition-colors hover:bg-surface-subtle",
          "truncate",
        )}
        onClick={() => setOpen(current => !current)}
        type="button"
      >
        <span aria-hidden className="shrink-0 text-sm">
          {active ? iconGlyph(active.icon) : "🗂"}
        </span>
        <span className="min-w-0 flex-1 truncate text-left">{label}</span>
        <ChevronDown className="size-3.5 shrink-0 text-ink-soft" />
      </button>

      {open
        ? (
            <MenuPanel className="absolute left-2 right-2 top-9 z-40 p-1" ref={menuRef} role="listbox">
              <SelectorOption
                active={filter.kind === "all"}
                label={t("selector.all")}
                onClick={() => choose({ kind: "all" })}
              >
                <MonitorSmartphone className="size-3.5" />
              </SelectorOption>
              <SelectorOption
                active={filter.kind === "device" && filter.desktopId === null}
                label={t("selector.local")}
                onClick={() => choose({ kind: "device", desktopId: null })}
              >
                <Monitor className="size-3.5" />
              </SelectorOption>
              {peers.map(peer => (
                <SelectorOption
                  active={filter.kind === "device" && filter.desktopId === peer.desktopId}
                  key={peer.desktopId}
                  label={peerBadgeText(peer, peer.desktopId)}
                  // A disconnected machine stays selectable: its rows are gone
                  // anyway, and disabling the entry would leave no way to see
                  // that it is merely offline rather than removed.
                  note={peer.connected ? undefined : t("statusDisconnected")}
                  onClick={() => choose({ kind: "device", desktopId: peer.desktopId })}
                >
                  <span aria-hidden>{iconGlyph(peer.icon)}</span>
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
      {note ? <span className="shrink-0 text-[10px] text-ink-muted">{note}</span> : null}
      {active ? <Check className="size-3.5 shrink-0" /> : null}
    </button>
  );
}
