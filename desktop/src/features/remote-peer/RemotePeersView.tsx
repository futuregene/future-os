import type { RemotePeer } from "./remotePeerClient";
import { ChevronDown } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { TextInput } from "../../components/ui/TextInput";
import { cn } from "../../lib/cn";
import { emitFutureEvent } from "../../lib/futureEvents";
import { iconGlyph, peerBadgeText } from "./peerIcons";
import { peersConnected, peerStatus, peerStatusKey, peerStatusTone } from "./peerStatus";
import {
  connectRemotePeer,
  disconnectRemotePeer,
  listRemotePeers,
  pairRemotePeer,
  unpairRemotePeer,
} from "./remotePeerClient";

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
 */
export function RemoteClientSection({
  onOpenSettings,
  onReconnectAll,
  onStartConversation,
  refreshNonce,
}: {
  /**
   * Open a new conversation on that host. Owned by the shell because it decides
   * what the user is looking at; this section only knows the machines.
   */
  onStartConversation: (desktopId: string) => void;
  /** Open that machine's settings, as a page of the window the user is in. */
  onOpenSettings: (peer: RemotePeer) => void;
  /**
  /**
   * Bring every paired machine up, the stopped ones included.
   *
   * Owned by the shell because the liveness pass is the shell's: this section
   * reads the machines, it does not decide when they come back. Re-reads the
   * list, so the rows below settle into their new state.
   */
  onReconnectAll: () => Promise<void>;
  /**
   * Bumped when something outside this section changed a machine — a rename on
   * its settings page — so the list re-reads instead of showing what it read
   * when the page was first opened.
   */
  refreshNonce?: number;
}) {
  const { t } = useTranslation("remotePeer");
  const [peers, setPeers] = useState<RemotePeer[]>([]);
  /**
   * Whether the first read has answered yet.
   *
   * The rows start empty, so "no machines are paired" and "the answer has not
   * arrived" looked identical — and the read goes through the backend's runtime
   * lock, which a host that has just dropped can hold for a while. The section
   * then said "no remote desktops yet" and quietly filled in: a machine the user
   * has paired looked *removed*. Saying it is still reading is the honest
   * reading, and the same distinction `peerStatus` draws between "offline" and
   * "still being reached".
   */
  const [loading, setLoading] = useState(true);
  const [invitation, setInvitation] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
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

  const refresh = useCallback(async () => {
    try {
      setPeers(await listRemotePeers());
    }
    catch (err) {
      // A read failure here is local (the credential book); the per-host
      // connection errors arrive on the peers themselves.
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      // Either way the question "is this a list, or is it still being read?"
      // has an answer now.
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh, refreshNonce]);

  async function run(desktopId: string, action: () => Promise<unknown>) {
    setBusy(desktopId);
    setError(null);
    try {
      await action();
      await refresh();
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(null);
    }
  }

  async function handleReconnectAll() {
    setBusy("__all__");
    setError(null);
    try {
      await onReconnectAll();
      await refresh();
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
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
    setError(null);
    try {
      await pairRemotePeer(link);
      // The link is single-use: keeping it in the box would invite the user to
      // press the button again and get a confusing failure.
      setInvitation("");
      // Paired: the machine it added is now a row below, which is what the user
      // came to see — so the form folds away behind the row it came from.
      setAddOpen(false);
      await refresh();
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(null);
    }
  }

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

        {error ? <p className="text-xs text-danger">{error}</p> : null}

        {/* The count is what the rows below add up to, and it is the answer to
            "why is a machine's conversation missing": a list that is short of
            one machine is the state the reader has to be able to see, and the
            app now holds every pairing open, so "some of them are not up" is a
            fact about this machine, not about their settings. */}
        {peers.length > 1
          ? (
              <div className="flex items-center justify-between gap-3">
                <p className="text-xs text-ink-muted" data-testid="peers-summary">
                  {t("peersSummary", {
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
                    onNewConversation={() => onStartConversation(peer.desktopId)}
                    onReconnect={() => void run(peer.desktopId, () => connectRemotePeer(peer.desktopId))}
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
  const status = peerStatus(peer);
  return { tone: peerStatusTone(status), label: t(peerStatusKey(status)) };
}

function PeerRow({
  busy,
  onDisconnect,
  onEdit,
  onNewConversation,
  onReconnect,
  onUnpair,
  peer,
}: {
  busy: boolean;
  onDisconnect: () => void;
  onEdit: () => void;
  onNewConversation: () => void;
  onReconnect: () => void;
  onUnpair: () => void;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const status = peerBadge(peer, t);
  // Starting a conversation needs the host's *agent*, not just its bridge: the
  // bridge can be reachable while every prompt fails. Offering the button there
  // would hand the user a box whose messages cannot go anywhere.
  const canConverse = peerStatus(peer) === "connected";
  // A machine the user turned off offers the way back; so does one that could
  // not be reached. Only a live connection offers to stop.
  const showDisconnect = peer.connected && !peer.stopped;

  return (
    <div className="flex items-center gap-3 rounded-lg border border-line-soft p-4" data-testid="peer-row">
      <span aria-hidden className="text-lg" data-testid="peer-icon">{iconGlyph(peer.icon)}</span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium text-ink">
            {peerBadgeText(peer, peer.desktopId)}
          </span>
          <Badge tone={status.tone}>{status.label}</Badge>
        </div>
        <div className="mt-0.5 truncate text-xs text-ink-muted">{peer.desktopId}</div>
        {/* A stopped machine's last failure is stale by definition: the socket
            went away because the user closed it. Saying both would read as a
            fault that is not there. */}
        {peer.error && !peer.stopped ? <div className="mt-1 text-xs text-danger">{peer.error}</div> : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {canConverse
          ? (
              <Button disabled={busy} onClick={onNewConversation} size="sm" variant="primary">
                {t("newConversation")}
              </Button>
            )
          : null}
        {showDisconnect
          ? <Button disabled={busy} onClick={onDisconnect} size="sm">{t("disconnect")}</Button>
          : <Button disabled={busy} onClick={onReconnect} size="sm" variant="primary">{t("connect")}</Button>}
        <Button disabled={busy} onClick={onEdit} size="sm" variant="ghost">{t("edit")}</Button>
        {/* Last, and marked: it is the one action here that removes the row it
            sits on, and the only one that is confirmed. */}
        <Button disabled={busy} onClick={onUnpair} size="sm" variant="danger-soft">{t("unpair")}</Button>
      </div>
    </div>
  );
}
