import type { RemotePeer } from "./remotePeerClient";
import { ChevronRight } from "lucide-react";
import { useEffect, useRef, useState } from "react";
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
 * action with a pre-chosen answer is the defect this dialog was added for. The
 * row it opens on is a *cursor* — the one the arrows and Enter act on — not an
 * answer already given: nothing happens until a row is opened.
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

  /**
   * The row drawn as the chosen one, counted among the rows that can be chosen.
   *
   * It starts on this machine, which is the row the dialog opens on. It is
   * state rather than the `:focus-visible` the row's own focus would paint,
   * because that pseudo-class is a guess about the input device: WebKit — the
   * desktop app's engine — does not match it for an element focused by script,
   * so the row this dialog had already chosen was drawn exactly like the others
   * and the list read as "nothing is selected, reach for the mouse". The state
   * paints the row; the caret follows the state, and a row the reader reaches
   * with Tab or a click moves the state back, so what looks chosen and what
   * Enter opens are never two different rows.
   */
  const [chosen, setChosen] = useState(0);
  // The key listener is installed once and reads the choice through this: keys
  // arrive faster than renders, and two presses must land two rows apart instead
  // of both measuring from the same starting point.
  const chosenRef = useRef(0);
  useEffect(() => {
    chosenRef.current = chosen;
  }, [chosen]);

  // The hands are already on the keyboard when this appears: it is what ⌘N
  // opens. So the list takes the caret on open, on its first row — this
  // machine, which can always take the work.
  useEffect(() => {
    chosenRows(listRef.current)[0]?.focus();
  }, []);

  /**
   * The arrows and Enter, listened for on the window rather than on the list.
   *
   * A list only sees a key its own subtree holds the caret for, and the state
   * above exists because that caret is the part that was not to be trusted: on
   * a dialog that opens over a screen whose composer wants the caret, the
   * keystroke has to work even when the focus never arrived. Nothing else in
   * the app answers an unmodified arrow while a modal is up (`keyboardOwners`
   * says so for the global shortcuts), so this claims only what it can see.
   *
   * Enter is answered here rather than left to the button's own activation: that
   * is what the key means in a list of this kind, and saying so is what makes it
   * testable without a browser. `preventDefault` keeps the same key from opening
   * the row a second time through its default action — and the footer's own
   * button keeps its Enter, so Cancel is still cancelled rather than pre-empted.
   *
   * `:not(:disabled)` skips a machine that cannot take the work instead of
   * landing on it and coming back refused. Such a row is still drawn: it is
   * paired, and the reader looking for it needs to see whether it is offline,
   * being reached, or driven by another device.
   */
  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const list = listRef.current;
      if (!list)
        return;
      const rows = chosenRows(list);
      if (rows.length === 0)
        return;

      if (event.key === "Enter") {
        const caret = document.activeElement;
        if (caret instanceof HTMLElement && caret.tagName === "BUTTON" && !list.contains(caret))
          return;
        event.preventDefault();
        rows[Math.min(chosenRef.current, rows.length - 1)]?.click();
        return;
      }
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp" && event.key !== "Home" && event.key !== "End")
        return;
      event.preventDefault();
      const at = chosenRef.current;
      const next = event.key === "Home"
        ? 0
        : event.key === "End"
          ? rows.length - 1
          : (at + (event.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length;
      chosenRef.current = next;
      setChosen(next);
      rows[next]?.focus();
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
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

  // This machine is the choice at 0, so the peers that can be chosen number from
  // 1 in the order they are drawn: the row painted as chosen is the row the
  // arrows counted to.
  let cursor = 0;
  const peerRows = peers.map((peer) => {
    const ready = peerDrivable(peer);
    return { peer, ready, note: ready ? undefined : heldNote(peer) ?? t(peerStatusKey(peerStatus(peer))), position: ready ? ++cursor : -1 };
  });

  return (
    <Dialog
      description={description}
      footer={<Button onClick={onClose} size="sm" variant="ghost">{tCommon("cancel")}</Button>}
      onClose={onClose}
      open
      title={title}
    >
      <div className="max-h-80 space-y-0.5 overflow-y-auto" ref={listRef}>
        {/* This machine leads: it is the answer a create action used to assume,
            and it is always able to take the work. */}
        <DeviceRow
          chosen={chosen === 0}
          glyph={<PeerIconGlyph className="size-4" id="desktop" />}
          label={t("selector.local")}
          onClick={onPickLocal}
          onFocus={() => setChosen(0)}
        />
        {peerRows.map(({ peer, ready, note, position }) => (
          // "Connected" is not enough: a machine another paired device drives
          // answers every heartbeat and refuses the work (`superseded`), so it is
          // listed with the reason and not offered as an answer.
          <DeviceRow
            chosen={chosen === position}
            disabled={!ready}
            glyph={<PeerIconGlyph className="size-4" id={peer.icon} />}
            key={peer.desktopId}
            label={peerBadgeText(peer, peer.desktopId)}
            note={note}
            onClick={() => onPickPeer(peer)}
            onFocus={() => setChosen(position)}
          />
        ))}
      </div>
    </Dialog>
  );
}

/** The rows the arrows walk: every machine that can take the work. */
function chosenRows(list: HTMLElement | null): HTMLButtonElement[] {
  if (!list)
    return [];
  return [...list.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
}

/**
 * One machine, as a row that opens it.
 *
 * Not a radio: a tap *is* the choice — it moves the dialog to that machine's
 * form — so the chevron says "this goes somewhere" rather than a tick that
 * would invite a separate confirmation. The chosen row is still drawn as chosen
 * (the field's place, and the ring a keyboard user looks for), because the
 * arrows and Enter act on it: a cursor nobody can see is a list that looks
 * answerable only by touch.
 */
function DeviceRow({
  chosen,
  disabled = false,
  glyph,
  label,
  note,
  onClick,
  onFocus,
}: {
  chosen: boolean;
  disabled?: boolean;
  glyph: React.ReactNode;
  label: string;
  note?: string;
  onClick: () => void;
  onFocus?: () => void;
}) {
  return (
    <button
      aria-label={label}
      className={cn(
        "flex w-full items-center gap-2 rounded-md px-2 py-2 text-left text-sm outline-none",
        disabled
          ? "cursor-not-allowed text-ink-muted"
          // The ring is inset because the scrolling box around the list would
          // otherwise clip it. It follows `chosen` rather than `:focus-visible`
          // — see the note on that state above.
          : chosen
            ? "bg-surface-subtle text-ink ring-2 ring-inset ring-focus"
            : "text-ink hover:bg-surface-subtle",
      )}
      data-chosen={chosen ? "true" : "false"}
      disabled={disabled}
      onClick={onClick}
      onFocus={onFocus}
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
