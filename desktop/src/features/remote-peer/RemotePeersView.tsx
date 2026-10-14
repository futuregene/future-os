import type { RemotePeer } from "./remotePeerClient";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
import { iconGlyph, peerBadgeText } from "./peerIcons";
import { peersConnected, peerStatus, peerStatusKey, peerStatusTone } from "./peerStatus";
import {
  connectRemotePeer,
  disconnectRemotePeer,
  listRemotePeers,
  pairRemotePeer,
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
 * naming, the icon, connection state, and unpairing.
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
   * Bring every paired machine up, the stopped ones included.
   *
   * Owned by the shell because the liveness pass is the shell's: this section
   * reads the machines, it does not decide when they come back. Re-reads the
   * list, so the rows below settle into their new state.
   */
  onReconnectAll: () => Promise<void>;
  /**
   * Bumped when something outside this section changed a machine (a rename or an
   * unpair on its settings page), so the list re-reads instead of showing what it
   * read when the page was first opened.
   */
  refreshNonce?: number;
}) {
  const { t } = useTranslation("remotePeer");
  const [peers, setPeers] = useState<RemotePeer[]>([]);
  const [invitation, setInvitation] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setPeers(await listRemotePeers());
    }
    catch (err) {
      // A read failure here is local (the credential book); the per-host
      // connection errors arrive on the peers themselves.
      setError(err instanceof Error ? err.message : String(err));
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
      <div className="space-y-6">
        {/* The paragraph that explains this direction lives on the hub, which
                  renders it as this section's subtitle. Rendering it here too put
                  the same sentence on screen twice. */}
        <div className="rounded-lg border border-line-soft bg-surface-subtle p-4">
          <label className="text-sm font-medium text-ink" htmlFor="peer-invitation">
            {t("addTitle")}
          </label>
          <p className="mt-1 text-xs leading-5 text-ink-muted">{t("addHint")}</p>
          <div className="mt-3 flex items-center gap-2">
            <TextInput
              id="peer-invitation"
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
          ? <p className="text-sm text-ink-soft">{t("empty")}</p>
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
                    peer={peer}
                  />
                ))}
              </div>
            )}
      </div>
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
  peer,
}: {
  busy: boolean;
  onDisconnect: () => void;
  onEdit: () => void;
  onNewConversation: () => void;
  onReconnect: () => void;
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
      </div>
    </div>
  );
}
