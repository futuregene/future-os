import type { RemotePeer } from "./remotePeerClient";
import { ChevronRight, Monitor } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { cn } from "../../lib/cn";
import { iconGlyph, peerBadgeText } from "./peerIcons";
import { peerStatus, peerStatusKey } from "./peerStatus";

/**
 * Which machine a new workspace is on.
 *
 * The workspace "+" used to open this app's own create-workspace form straight
 * away, whose picker browses *this* computer's disk. With another machine
 * paired that answered a question the user was not asked — they were already
 * looking at a list holding two machines' conversations, and the folder being
 * chosen belonged to one of them. So the machine is picked first, exactly as
 * the phone's new-conversation dialog does, and the form that follows belongs
 * to the machine it names.
 *
 * Machines that cannot take work are listed rather than hidden: they are still
 * paired, and the reader looking for one needs to see that it is offline, being
 * reached, or whose agent is restarting — not conclude it was removed.
 */
export function WorkspaceDeviceDialog({
  onClose,
  onPickLocal,
  onPickPeer,
  peers,
}: {
  onClose: () => void;
  onPickLocal: () => void;
  onPickPeer: (peer: RemotePeer) => void;
  peers: RemotePeer[];
}) {
  const { t } = useTranslation("remotePeer");
  const { t: tCommon } = useTranslation("common");

  return (
    <Dialog
      description={t("chooseDeviceHint")}
      footer={<Button onClick={onClose} size="sm" variant="ghost">{tCommon("cancel")}</Button>}
      onClose={onClose}
      open
      title={t("chooseDevice")}
    >
      <div className="max-h-80 space-y-0.5 overflow-y-auto">
        {/* This machine leads: it is the answer the old form assumed, and it is
            always able to take the workspace. */}
        <DeviceRow
          glyph={<Monitor aria-hidden className="size-4" />}
          label={t("selector.local")}
          onClick={onPickLocal}
        />
        {peers.map((peer) => {
          const status = peerStatus(peer);
          const ready = status === "connected";
          return (
            <DeviceRow
              disabled={!ready}
              glyph={<span aria-hidden className="text-sm">{iconGlyph(peer.icon)}</span>}
              key={peer.desktopId}
              label={peerBadgeText(peer, peer.desktopId)}
              note={ready ? undefined : t(peerStatusKey(status))}
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
        ? <span className="shrink-0 text-[10px] text-ink-muted" data-testid="option-note">{note}</span>
        : null}
      {disabled ? null : <ChevronRight aria-hidden className="size-3.5 shrink-0 text-ink-muted" />}
    </button>
  );
}
