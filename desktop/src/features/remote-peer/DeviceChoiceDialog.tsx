import type { KeyboardEvent } from "react";
import type { RemotePeer } from "./remotePeerClient";
import { ChevronRight } from "lucide-react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { cn } from "../../lib/cn";
import { PeerIconGlyph } from "./PeerIconGlyph";
import { peerBadgeText } from "./peerIcons";
import { peerDrivable, peerHeld, peerHolderName, peerStatus, peerStatusKey } from "./peerStatus";

/**
 * Which machine something new belongs to.
 *
 * The step in front of two flows that both make something on one machine's
 * disk — a workspace, and a conversation. It exists because the list above it
 * can hold several machines' rows: a create action that skipped this question
 * answered it silently, and the thing it made landed on this computer whatever
 * the reader was looking at.
 *
 * What is deliberately *not* here: a "no particular machine" entry, or a
 * default that acts without a tap. The machine is the question, and a create
 * action with a pre-chosen answer is the defect this dialog was added for.
 *
 * Machines that cannot take work are listed rather than hidden: they are still
 * paired, and the reader looking for one needs to see that it is offline, being
 * reached, or whose agent is restarting — not conclude it was removed.
 */
export function DeviceChoiceDialog({
  description,
  onClose,
  onPickLocal,
  onPickPeer,
  peers,
  title,
}: {
  description: string;
  onClose: () => void;
  onPickLocal: () => void;
  onPickPeer: (peer: RemotePeer) => void;
  peers: RemotePeer[];
  title: string;
}) {
  const { t } = useTranslation("remotePeer");
  const { t: tCommon } = useTranslation("common");
  const listRef = useRef<HTMLDivElement>(null);

  // The hands are already on the keyboard when this appears: it is what ⌘N
  // opens. So the list takes the caret on open, on its first row — this
  // machine, which can always take the work — and the arrows and Enter then
  // answer the question without a trip to the mouse. A dialog that opened with
  // the caret nowhere asked the keyboard user to reach for the pointer to say
  // which computer they meant.
  useEffect(() => {
    listRef.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, []);

  /**
   * Why a machine cannot take this, when the reason is another paired device.
   *
   * Its status is "connected" — the link is up — so the status note would say
   * nothing at all, and the reader would be looking at an enabled-looking row
   * whose work comes back refused. The sentence is the picker's own.
   */
  function heldNote(peer: RemotePeer): string | undefined {
    if (!peerHeld(peer))
      return undefined;
    const name = peerHolderName(peer);
    return name ? t("heldBy", { name }) : t("heldByUnknown");
  }

  return (
    <Dialog
      description={description}
      footer={<Button onClick={onClose} size="sm" variant="ghost">{tCommon("cancel")}</Button>}
      onClose={onClose}
      open
      title={title}
    >
      <div
        className="max-h-80 space-y-0.5 overflow-y-auto"
        onKeyDown={handleRowKeyDown}
        ref={listRef}
      >
        {/* This machine leads: it is the answer a create action used to assume,
            and it is always able to take the work. */}
        <DeviceRow
          glyph={<PeerIconGlyph className="size-4" id="desktop" />}
          label={t("selector.local")}
          onClick={onPickLocal}
        />
        {peers.map((peer) => {
          // "Connected" is not enough: a machine another paired device drives
          // answers every heartbeat and refuses the work (`superseded`), so it is
          // listed with the reason and not offered as an answer.
          const ready = peerDrivable(peer);
          const status = peerStatus(peer);
          return (
            <DeviceRow
              disabled={!ready}
              glyph={<PeerIconGlyph className="size-4" id={peer.icon} />}
              key={peer.desktopId}
              label={peerBadgeText(peer, peer.desktopId)}
              note={ready ? undefined : heldNote(peer) ?? t(peerStatusKey(status))}
              onClick={() => onPickPeer(peer)}
            />
          );
        })}
      </div>
    </Dialog>
  );
}

/**
 * Walk the rows with the arrows and open the one Enter lands on.
 *
 * The same keys the rail's menus answer to (`ActivityRailMenus`), for the same
 * reason: this list is a list, and the user who opened it from the keyboard
 * expects to finish there. Focus is the selection — the arrows move the caret
 * and the row's own highlight follows it in CSS — so there is no second state
 * to keep in step with what is on screen.
 *
 * `:not(:disabled)` skips a machine that cannot take the work instead of
 * landing on it and coming back refused. Such a row is still drawn: it is
 * paired, and the reader looking for it needs to see whether it is offline,
 * being reached, or driven by another device.
 *
 * Enter is answered here rather than left to the button's own activation: that
 * is what the key means in a list of this kind, and saying so is what makes it
 * testable without a browser. `preventDefault` keeps the same key from opening
 * the row a second time through its default action.
 */
function handleRowKeyDown(event: KeyboardEvent<HTMLDivElement>) {
  const rows = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
  if (rows.length === 0)
    return;
  if (event.key === "Enter") {
    const row = rows.find(candidate => candidate === document.activeElement);
    if (!row)
      return;
    event.preventDefault();
    row.click();
    return;
  }
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End")
    return;
  event.preventDefault();
  if (event.key === "Home" || event.key === "End") {
    rows[event.key === "Home" ? 0 : rows.length - 1]?.focus();
    return;
  }
  const at = rows.indexOf(document.activeElement as HTMLButtonElement);
  const delta = event.key === "ArrowDown" ? 1 : -1;
  // Nothing of the list has the caret (the focus is on the footer, say): "down"
  // starts at the top and "up" at the bottom, rather than at whichever end the
  // arithmetic happens to land on.
  const next = at < 0
    ? (delta > 0 ? 0 : rows.length - 1)
    : (at + delta + rows.length) % rows.length;
  rows[next]?.focus();
}

/**
 * One machine, as a row that opens it.
 *
 * Not a radio: a tap *is* the choice — it moves the dialog to that machine's
 * form — so the chevron says "this goes somewhere" rather than a tick that
 * would invite a separate confirmation.
 */
function DeviceRow({
  disabled = false,
  glyph,
  label,
  note,
  onClick,
}: {
  disabled?: boolean;
  glyph: React.ReactNode;
  label: string;
  note?: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2 py-2 text-left text-sm",
        disabled
          ? "cursor-not-allowed text-ink-muted"
          // The focused row *is* the chosen one while the list is walked
          // (`handleRowKeyDown` moves the focus, not a highlight of its own),
          // so the highlight has to follow the focus. Inset, because the ring
          // would otherwise be clipped by the scrolling box around the list.
          : "text-ink hover:bg-surface-subtle focus-visible:bg-surface-subtle focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-focus",
      )}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      {/* One slot for both kinds of mark. Both are 16px line icons from the
          shared peer-icon set now, so the reserved box is what keeps every
          row's name starting at the same x rather than what evens out two
          different glyph widths. */}
      <span aria-hidden className="flex size-5 shrink-0 items-center justify-center leading-none">
        {glyph}
      </span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {note
        ? <span className="shrink-0 text-3xs text-ink-muted" data-testid="option-note">{note}</span>
        : null}
      {disabled ? null : <ChevronRight aria-hidden className="size-3.5 shrink-0 text-ink-muted" />}
    </button>
  );
}
