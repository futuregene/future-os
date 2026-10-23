import type { RemotePeer } from "./remotePeerClient";
import { ChevronDown } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { TextInput } from "../../components/ui/TextInput";
import { cn } from "../../lib/cn";
import { emitFutureEvent } from "../../lib/futureEvents";
import { PeerIconChip } from "./PeerIconChip";
import { iconGlyph, peerBadgeText } from "./peerIcons";
import {
  peerHeld,
  peerHolderName,
  peerLabelKey,
  peerLabelTone,
  peerRevoked,
  peersConnected,
} from "./peerStatus";
import {
  connectRemotePeer,
  disconnectRemotePeer,
  pairRemotePeer,
  takeoverRemotePeer,
  unpairRemotePeer,
} from "./remotePeerClient";
import { remotePeerErrorText } from "./remotePeerError";

/**
 * "Connect to another computer" — the client half of the Remote page.
 *
 * A section rather than a screen: the hub owns the page shell and the heading.
 *
 * Deliberately a management surface, not a conversation list. Conversations from
 * every host live in the shared conversation list (with a source badge), so that
 * there is exactly one place to look for "a conversation"; duplicating a second
 * list here would make the user pick between two answers to the same question.
 * What is unique to this section is everything about the *machines*: pairing,
 * naming, the icon, connection state, and unpairing — which lives on the row of
 * the machine it removes, because that is where the user is when they decide
 * they no longer want it. (It was a row on that machine's settings page until it
 * moved here: an action the user has to go and find on another page is one they
 * conclude does not exist.)
 *
 * The rows are drawn from the shell's peer list rather than from a read of this
 * section's own (`useRemotePeers`). That list is the live one: it is polled, and
 * the backend's `control` packet re-reads it the moment another paired device
 * takes a machine over — which is the one change on this page that happens
 * elsewhere, while the user is looking at it. A private copy here was read
 * once, when the page opened, so a takeover made on the other machine left this
 * row saying "Connected" with nothing to take, and the reverse (control coming
 * back) stayed wrong too, until the user left the page and returned. Reading
 * the list twice also meant two answers to the same question could disagree on
 * screen.
 */
export function RemoteClientSection({
  error,
  loading,
  onOpenSettings,
  onReconnectAll,
  onRefresh,
  peers,
}: {
  /**
   * Why the peer list could not be read — the shell's own read, whose failure is
   * the difference between "no machines are paired" and "I could not find out".
   */
  error: string | null;
  /**
   * Whether the shell's first read has answered yet.
   *
   * The rows start empty, so "no machines are paired" and "the answer has not
   * arrived" would look identical — and the read goes through the backend's
   * runtime lock, which a host that has just dropped can hold for a while. The
   * section would then say "no remote desktops yet" and quietly fill in: a
   * machine the user has paired looked *removed*. Saying it is still reading is
   * the honest reading, and the same distinction `peerStatus` draws between
   * "offline" and "still being reached".
   */
  loading: boolean;
  /** Open that machine's settings, as a page of the window the user is in. */
  onOpenSettings: (peer: RemotePeer) => void;
  /**
   * Bring every paired machine up, the stopped ones included.
   *
   * Owned by the shell because the liveness pass is the shell's: this section
   * reads the machines, it does not decide when they come back. Re-reads the
   * list, so the rows below settle into their new state.
   */
  onReconnectAll: () => Promise<void>;
  /**
   * Re-read the peer list after something changed a machine from this section
   * (a pairing, a connect, a takeover, an unpair). The read belongs to the
   * shell — see the section comment — so an action asks for it rather than
   * keeping a second list of its own.
   */
  onRefresh: () => Promise<void>;
  /** The paired machines, as the shell's live list has them. */
  peers: readonly RemotePeer[];
}) {
  const { t } = useTranslation("remotePeer");
  const [invitation, setInvitation] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  /**
   * The failure of one action taken on this page, as opposed to the list read.
   *
   * Kept apart from `error` because they answer different questions — "that
   * command did not go through" against "I could not read your machines" — and
   * a successful action must not clear a read failure it did not fix.
   */
  const [actionError, setActionError] = useState<string | null>(null);
  /** The machine whose unpair confirmation is open, if any. */
  const [unpairTarget, setUnpairTarget] = useState<RemotePeer | null>(null);
  /**
   * Whether the pairing form is unfolded.
   *
   * Folded to start: the machines below are what the card is for, and the form
   * is one errand per machine. See where it is rendered for the widths this
   * buys back.
   */
  const [addOpen, setAddOpen] = useState(false);

  async function run(desktopId: string, action: () => Promise<unknown>) {
    setBusy(desktopId);
    setActionError(null);
    try {
      await action();
      // The rows are the shell's list, so the read is what makes the action
      // visible: without it a Connect would leave its own row looking untouched
      // for a whole poll.
      await onRefresh();
    }
    catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(null);
    }
  }

  async function handleReconnectAll() {
    setBusy("__all__");
    setActionError(null);
    try {
      await onReconnectAll();
      await onRefresh();
    }
    catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(null);
    }
  }

  /**
   * Unpair one machine, once the user has confirmed it.
   *
   * The pairing is gone locally whatever the platform says, so a revoke the
   * platform never received is *reported* instead of being claimed as a clean
   * removal — the row disappears either way.
   */
  async function handleUnpair(peer: RemotePeer) {
    setUnpairTarget(null);
    await run(peer.desktopId, async () => {
      const pending = await unpairRemotePeer(peer.desktopId);
      if (pending)
        emitFutureEvent("toast", { message: t("unpairPendingWarning"), tone: "info" });
    });
  }

  async function handlePair() {
    const link = invitation.trim();
    if (!link)
      return;
    setBusy("__pair__");
    setActionError(null);
    try {
      await pairRemotePeer(link);
      // The link is single-use: keeping it in the box would invite the user to
      // press the button again and get a confusing failure.
      setInvitation("");
      // Paired: the machine it added is now a row below, which is what the user
      // came to see — so the form folds away behind the row it came from.
      setAddOpen(false);
      await onRefresh();
    }
    catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(null);
    }
  }

  // Machines another paired device drives: connected, and not contributing a
  // single conversation to the list above.
  const heldCount = peers.filter(peerHeld).length;
  // The action that was just attempted is the more useful of the two: it is the
  // one the user caused, and the read failure under it would be the older fact.
  const shownError = actionError ?? error;

  return (
    <>
      <div className="space-y-4">
        {/* The paragraph that explains this direction lives on the hub, which
                  renders it as this section's subtitle. Rendering it here too put
                  the same sentence on screen twice.

                  The form itself is folded away. Pairing is one errand per machine;
                  the machines the user already has are what this card is for, and the
                  open form stood above them at 158px — a fifth of the page — with two
                  sentences of prose about a text box. The row says what is behind it,
                  and the instructions are read when the box is actually in front of
                  the user. */}
        <div className="rounded-lg border border-line-soft bg-surface-subtle">
          <button
            aria-expanded={addOpen}
            className="flex w-full items-center gap-3 p-4 text-left"
            onClick={() => setAddOpen(open => !open)}
            type="button"
          >
            <span className="min-w-0 flex-1 text-sm font-medium text-ink">{t("addTitle")}</span>
            <ChevronDown
              aria-hidden
              className={cn("size-4 shrink-0 text-ink-muted transition-transform", addOpen && "rotate-180")}
            />
          </button>
          {addOpen
            ? (
                <div className="px-4 pb-4">
                  <p className="text-xs leading-5 text-ink-muted">{t("addHint")}</p>
                  <div className="mt-3 flex items-center gap-2">
                    <TextInput
                      aria-label={t("addTitle")}
                      onChange={event => setInvitation(event.target.value)}
                      placeholder="futureos://remote/pair?…"
                      value={invitation}
                    />
                    <Button
                      disabled={!invitation.trim() || busy !== null}
                      onClick={() => void handlePair()}
                      size="sm"
                      variant="primary"
                    >
                      {busy === "__pair__" ? t("adding") : t("add")}
                    </Button>
                  </div>
                  {/* Replacing the phone pairing on the other machine is a real
                        consequence of this action, so it is stated before, not after. */}
                  <p className="mt-2 text-xs leading-5 text-ink-muted">{t("slotWarning")}</p>
                </div>
              )
            : null}
        </div>

        {shownError
          ? (
              // The backend's own words are diagnostics ("Remote handshake
              // failed: no responders: no responders"); the sentence is the
              // UI's. The raw text stays as the tooltip so support can still
              // see what actually failed.
              <p className="text-xs text-danger" title={shownError}>{remotePeerErrorText(shownError, t)}</p>
            )
          : null}

        {/* The count is what the rows below add up to, and it is the answer to
            "why is a machine's conversation missing": a list that is short of
            one machine is the state the reader has to be able to see, and the
            app now holds every pairing open, so "some of them are not up" is a
            fact about this machine, not about their settings.

            A machine another paired device drives is *connected* and still
            missing from the list, so a count that stopped at "N of M connected"
            would call that state complete. The held machines are counted in the
            same sentence for the same reason. */}
        {peers.length > 1
          ? (
              <div className="flex items-center justify-between gap-3">
                <p className="text-xs text-ink-muted" data-testid="peers-summary">
                  {heldCount > 0
                    ? t("peersSummaryHeld", {
                        connected: peersConnected(peers).connected,
                        held: heldCount,
                        total: peers.length,
                      })
                    : t("peersSummary", {
                        connected: peersConnected(peers).connected,
                        total: peers.length,
                      })}
                </p>
                <Button
                  disabled={busy !== null}
                  onClick={() => void handleReconnectAll()}
                  size="sm"
                >
                  {busy === "__all__" ? t("connecting") : t("reconnectAll")}
                </Button>
              </div>
            )
          : null}

        {peers.length === 0
          ? (
              <p className="text-sm text-ink-soft" data-testid={loading ? "peers-loading" : undefined}>
                {loading ? t("loading") : t("empty")}
              </p>
            )
          : (
              <div className="space-y-2">
                {peers.map(peer => (
                  <PeerRow
                    busy={busy === peer.desktopId || busy === "__all__"}
                    key={peer.desktopId}
                    onDisconnect={() => void run(peer.desktopId, () => disconnectRemotePeer(peer.desktopId))}
                    onEdit={() => onOpenSettings(peer)}
                    onReconnect={() => void run(peer.desktopId, () => connectRemotePeer(peer.desktopId))}
                    onTakeOver={() => void run(peer.desktopId, () => takeoverRemotePeer(peer.desktopId))}
                    onUnpair={() => setUnpairTarget(peer)}
                    peer={peer}
                  />
                ))}
              </div>
            )}
      </div>
      {/*
        * `Dialog` rather than the shared `ConfirmDeleteDialog`: its confirm button
        * reads "Delete", and unpairing a computer deletes nothing the user made.
        * The other machine keeps all of its data; what is removed here is one
        * pairing, which is what this button says.
        */}
      {unpairTarget
        ? (
            <Dialog
              description={t("unpairConfirmDesc", {
                name: unpairTarget.name ?? unpairTarget.desktopId,
              })}
              footer={(
                <>
                  <Button onClick={() => setUnpairTarget(null)} variant="ghost">
                    {t("cancel")}
                  </Button>
                  <Button
                    disabled={busy === unpairTarget.desktopId}
                    onClick={() => void handleUnpair(unpairTarget)}
                    variant="danger"
                  >
                    {t("unpair")}
                  </Button>
                </>
              )}
              onClose={() => setUnpairTarget(null)}
              open
              title={t("unpairConfirmTitle")}
            >
              <span />
            </Dialog>
          )
        : null}
    </>
  );
}

function peerBadge(peer: RemotePeer, t: (key: string) => string): { tone: "neutral" | "accent" | "warning" | "danger"; label: string } {
  // Not `peerStatus` alone: a machine answering while another paired device
  // drives it is connected on the transport and unusable in fact, and the word
  // here is what explains its conversations missing from the list.
  return { tone: peerLabelTone(peer), label: t(peerLabelKey(peer)) };
}

function PeerRow({
  busy,
  onDisconnect,
  onEdit,
  onReconnect,
  onTakeOver,
  onUnpair,
  peer,
}: {
  busy: boolean;
  onDisconnect: () => void;
  onEdit: () => void;
  onReconnect: () => void;
  /** Ask this machine to let this desktop drive it (it is in standby). */
  onTakeOver: () => void;
  onUnpair: () => void;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const status = peerBadge(peer, t);
  // A machine the user turned off offers the way back; so does one that could
  // not be reached. Only a live connection offers to stop.
  const showDisconnect = peer.connected && !peer.stopped;
  const held = peerHeld(peer);
  // The pairing is gone on the other side: the platform refuses this side's
  // credential every time, so the row offers the way out instead of the way in.
  const revoked = peerRevoked(peer);
  // Named when the controller declared a name; the generic sentence otherwise.
  const holderName = peerHolderName(peer);

  return (
    <div className="flex items-center gap-3 rounded-lg border border-line-soft p-4" data-testid="peer-row">
      <span data-testid="peer-icon"><PeerIconChip glyph={iconGlyph(peer.icon)} variant="row" /></span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium text-ink">
            {peerBadgeText(peer, peer.desktopId)}
          </span>
          <Badge tone={status.tone}>{status.label}</Badge>
        </div>
        <div className="mt-0.5 truncate text-xs text-ink-muted">{peer.desktopId}</div>
        {/* Why this machine's conversations are not in the list. Said here, on
            the machine's own row, because that is where the reader is when they
            ask it — the same sentence the phone's sheet shows. */}
        {held
          ? (
              <div className="mt-1 text-xs text-warning" data-testid="peer-held">
                {holderName ? t("heldBy", { name: holderName }) : t("heldByUnknown")}
              </div>
            )
          : null}
        {/* A stopped machine's last failure is stale by definition: the socket
            went away because the user closed it. Saying both would read as a
            fault that is not there. For a machine that is up on paper but not
            answering, the sentence is still the UI's: `peer.error` is the
            runtime's diagnostic text. */}
        {peer.error && !peer.stopped
          ? <div className="mt-1 text-xs text-danger" title={peer.error}>{remotePeerErrorText(peer.error, t)}</div>
          : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {/* Control comes back without a reconnection, and a machine in standby
            is the one thing this page exists for: its conversations are missing
            from the list until someone takes it. Offered before Disconnect
            because it is the action the reader came for. */}
        {held
          ? (
              <Button disabled={busy} onClick={onTakeOver} size="sm" variant="primary">
                {t("takeOver")}
              </Button>
            )
          : null}
        {revoked
          ? null
          : showDisconnect
            ? <Button disabled={busy} onClick={onDisconnect} size="sm">{t("disconnect")}</Button>
            : <Button disabled={busy} onClick={onReconnect} size="sm" variant="primary">{t("connect")}</Button>}
        {/* Settings read and write the *other* machine, and a machine another
            device drives will not answer them. Offered, it either failed with a
            transport error or — worse — opened a page whose edits went to a
            host that ignores this desktop. Disabled with the reason on hover: a
            disabled control with no explanation reads as a broken app. The
            title sits on the wrapper because a disabled button receives no
            pointer events of its own. */}
        <span title={held ? t("takeOverHint") : undefined}>
          <Button disabled={busy || held || revoked} onClick={onEdit} size="sm">{t("edit")}</Button>
        </span>
        {/* Last, and marked: it is the one action here that removes the row it
            sits on, and the only one that is confirmed. */}
        <Button disabled={busy} onClick={onUnpair} size="sm" variant="danger-soft">{t("unpair")}</Button>
      </div>
    </div>
  );
}
