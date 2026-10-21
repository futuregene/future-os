import type { RemotePeer } from "./remotePeerClient";
import { ChevronRight, Monitor } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { cn } from "../../lib/cn";
import { iconGlyph, peerBadgeText } from "./peerIcons";
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
      <div className="max-h-80 space-y-0.5 overflow-y-auto">
        {/* This machine leads: it is the answer a create action used to assume,
            and it is always able to take the work. */}
        <DeviceRow
          glyph={<Monitor aria-hidden className="size-4" />}
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
              glyph={<span aria-hidden className="text-sm">{iconGlyph(peer.icon)}</span>}
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
          : "text-ink hover:bg-surface-subtle",
      )}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <span className="shrink-0">{glyph}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {note
        ? <span className="shrink-0 text-3xs text-ink-muted" data-testid="option-note">{note}</span>
        : null}
      {disabled ? null : <ChevronRight aria-hidden className="size-3.5 shrink-0 text-ink-muted" />}
    </button>
  );
}
