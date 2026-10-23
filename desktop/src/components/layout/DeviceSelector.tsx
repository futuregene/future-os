import type { DeviceFilter } from "../../features/remote-peer/mergeConversations";
import type { RemotePeer } from "../../features/remote-peer/remotePeerClient";
import { Check, ChevronDown, MonitorSmartphone } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { PeerIconGlyph } from "../../features/remote-peer/PeerIconGlyph";
import { peerBadgeText } from "../../features/remote-peer/peerIcons";
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
  onTakeControl,
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
  /**
   * Ask a machine another paired device drives to let this one take it.
   *
   * The picker is where the user comes to ask why a machine's conversations are
   * missing from the merged list, so it is also where the way back belongs: the
   * Remote page spent a trip being the only place a takeover could be made. A
   * caller with nothing to offer omits this, and no button is drawn rather than
   * one that does nothing.
   */
  onTakeControl?: (desktopId: string) => Promise<void> | void;
  peers: RemotePeer[];
}) {
  const { t } = useTranslation("remotePeer");
  const [open, setOpen] = useState(false);
  // The machine whose takeover is in flight, so only that row's button shows
  // the wait and the rest of the list stays usable.
  const [takingOver, setTakingOver] = useState<string | null>(null);
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

  async function takeOver(desktopId: string) {
    if (!onTakeControl || takingOver)
      return;
    setTakingOver(desktopId);
    try {
      await onTakeControl(desktopId);
    }
    catch {
      // The row keeps the state it had: a machine that refuses is still held,
      // which is the truth, and a spinner left on is worse than a retry. The
      // machine's own refusal is what the user sees if it keeps refusing.
    }
    finally {
      setTakingOver(null);
    }
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
          // `text-13`, the rail's content tier: this control shares its row with
          // the search field, and at `text-xs` the row drew 12px and 14px 2px
          // apart.
          "flex items-center gap-1.5 rounded-md border border-line-soft px-1.5 text-13 font-medium text-ink transition-colors hover:bg-surface-subtle",
          compact ? "h-7 max-w-30" : "h-8 w-full gap-2 px-2",
        )}
        onClick={() => setOpen(current => !current)}
        type="button"
      >
        <GlyphSlot>{filterIcon(filter, peers)}</GlyphSlot>
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
              {peers.map((peer) => {
                const held = peerHeld(peer);
                return (
                  <SelectorOption
                    active={filter.kind === "device" && filter.desktopId === peer.desktopId}
                    // Taking a machine back is offered on its own entry, in the
                    // list the user is already reading, rather than only on the
                    // Remote page: a machine in standby answers but contributes
                    // no conversations, so this entry is where the reader learns
                    // it is held — and where the way out of that belongs.
                    action={held && onTakeControl
                      ? (
                          <button
                            aria-label={`${t("takeOver")} ${peerBadgeText(peer, peer.desktopId)}`}
                            className="inline-flex h-5 shrink-0 items-center whitespace-nowrap rounded-full border border-warning-line bg-warning-soft px-1.5 text-3xs font-medium text-warning transition-colors hover:brightness-95 disabled:opacity-60"
                            disabled={takingOver !== null}
                            onClick={() => void takeOver(peer.desktopId)}
                            type="button"
                          >
                            {takingOver === peer.desktopId ? "…" : t("takeOver")}
                          </button>
                        )
                      : undefined}
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
                );
              })}
              <div className="my-1 h-px bg-line-soft" />
              <button
                className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-13 text-accent hover:bg-surface-subtle"
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
 * The one column every mark in this control is drawn in.
 *
 * The marks are not the same kind of thing: "all devices" and "this computer"
 * are 14px line icons, while a paired machine carries an emoji the user picked,
 * and a colour emoji's advance is a third wider than a line icon's box at the
 * same font size. Left to their own widths the entries started their names at
 * three different x's — the list read as two columns, with the emoji hanging
 * into the first one. A reserved, centred slot pins the column, so every name
 * begins where the one above it does and the icons line up with each other
 * rather than each with its own glyph's edge.
 */
function GlyphSlot({ children }: { children: React.ReactNode }) {
  return (
    <span aria-hidden className="flex size-4 shrink-0 items-center justify-center leading-none">
      {children}
    </span>
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
    return <PeerIconGlyph className="size-3.5" id="desktop" />;
  return <PeerIconGlyph className="size-3.5" id={peers.find(peer => peer.desktopId === filter.desktopId)?.icon} />;
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
  action,
  active,
  children,
  label,
  note,
  onClick,
}: {
  /**
   * A control drawn at the entry's right edge, beside its name.
   *
   * A sibling of the entry's own button, not a child: an entry is one button
   * ("pick this machine"), and a second control nested inside it would make
   * both the markup and the click target ambiguous — the same reason the rail's
   * rows put their actions outside their full-row target.
   */
  action?: React.ReactNode;
  active: boolean;
  children: React.ReactNode;
  label: string;
  note?: string;
  onClick: () => void;
}) {
  return (
    // `role="none"`: the wrapper exists to place the action beside the entry,
    // not to be one. A listbox's children are its options, and an unnamed div in
    // between would put a group-like node there.
    <div className="relative" role="none">
      <button
        aria-label={label}
        aria-selected={active}
        className={cn(
          "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-13",
          active ? "bg-accent-soft font-medium text-accent" : "text-ink hover:bg-surface-subtle",
        )}
        onClick={onClick}
        role="option"
        type="button"
      >
        <GlyphSlot>{children}</GlyphSlot>
        {action
          ? (
              // The note moves under the name when an action shares the line,
              // the arrangement the phone's sheet uses: the action takes the
              // width the sentence kept for itself, and a truncated "In use
              // by…" would name nobody. The name keeps the width left over.
              <span className="min-w-0 flex-1">
                {/* `h-5` pins the name's line to the height of the action beside
                    it, so the note below starts exactly where the action ends
                    instead of under it. */}
                <span className="flex h-5 items-center gap-2 pr-20">
                  <span className="min-w-0 flex-1 truncate">{label}</span>
                  {active ? <Check className="size-3.5 shrink-0" /> : null}
                </span>
                {note
                  ? <span className="block truncate text-3xs text-ink-muted" data-testid="option-note">{note}</span>
                  : null}
              </span>
            )
          : (
              <>
                <span className="min-w-0 flex-1 truncate">{label}</span>
                {note ? <span className="shrink-0 text-3xs text-ink-muted" data-testid="option-note">{note}</span> : null}
                {active ? <Check className="size-3.5 shrink-0" /> : null}
              </>
            )}
      </button>
      {action ? <span className="absolute right-1.5 top-1.5 flex items-center">{action}</span> : null}
    </div>
  );
}
