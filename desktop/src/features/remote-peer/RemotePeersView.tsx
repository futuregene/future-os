import type { RemotePeer } from "./remotePeerClient";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { TextInput } from "../../components/ui/TextInput";
import { iconGlyph, peerBadgeText } from "./peerIcons";
import {
  connectRemotePeer,
  disconnectRemotePeer,
  listRemotePeers,
  pairRemotePeer,
  setRemotePeerLabel,
  unpairRemotePeer,
} from "./remotePeerClient";
import { RemotePeerSettings } from "./RemotePeerSettings";

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
  onOpenTasks,
  onStartConversation,
}: {
  /**
   * Open a new conversation on that host. Owned by the shell because it decides
   * what the user is looking at; this section only knows the machines.
   */
  onStartConversation: (desktopId: string) => void;
  /**
   * Open that host's tasks. Handled by the hub rather than here because the
   * task editor needs the whole column.
   */
  onOpenTasks: (peer: RemotePeer) => void;
}) {
  const { t } = useTranslation("remotePeer");
  const [peers, setPeers] = useState<RemotePeer[]>([]);
  const [invitation, setInvitation] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  /**
   * The host whose settings page is open, if any.
   *
   * A separate level rather than an accordion inside the list row: those
   * settings belong to a different machine, and presenting them inline next to
   * this machine's own state is how "auto-upgrade skills" becomes ambiguous.
   */
  const [settingsFor, setSettingsFor] = useState<string | null>(null);
  const [unpairTarget, setUnpairTarget] = useState<RemotePeer | null>(null);
  const settingsPeer = settingsFor
    ? peers.find(peer => peer.desktopId === settingsFor)
    : undefined;

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
  }, [refresh]);

  async function run(desktopId: string, action: () => Promise<unknown>) {
    setBusy(desktopId);
    setError(null);
    setWarning(null);
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

  async function handlePair() {
    const link = invitation.trim();
    if (!link)
      return;
    setBusy("__pair__");
    setError(null);
    setWarning(null);
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

  async function handleUnpair(peer: RemotePeer) {
    await run(peer.desktopId, async () => {
      const pending = await unpairRemotePeer(peer.desktopId);
      // Local pairing is already gone; say so when the platform-side revoke
      // could not be delivered instead of reporting a clean removal.
      if (pending)
        setWarning(t("unpairPendingWarning"));
    });
    setUnpairTarget(null);
  }

  return (
    <>
      {settingsPeer
        ? (
            <RemotePeerSettings
              onBack={() => setSettingsFor(null)}
              onChanged={() => void refresh()}
              onOpenTasks={() => onOpenTasks(settingsPeer)}
              onUnpair={() => setUnpairTarget(settingsPeer)}
              onUpdateLabel={patch => void run(settingsPeer.desktopId, () => setRemotePeerLabel(settingsPeer.desktopId, patch))}
              peer={settingsPeer}
            />
          )
        : (
            <div className="space-y-6">
              <p className="text-sm text-ink-muted">{t("description")}</p>

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
              {warning ? <p className="text-xs text-ink-muted">{warning}</p> : null}

              {peers.length === 0
                ? <p className="text-sm text-ink-soft">{t("empty")}</p>
                : (
                    <div className="space-y-2">
                      {peers.map(peer => (
                        <PeerRow
                          busy={busy === peer.desktopId}
                          key={peer.desktopId}
                          onDisconnect={() => void run(peer.desktopId, () => disconnectRemotePeer(peer.desktopId))}
                          onEdit={() => setSettingsFor(peer.desktopId)}
                          onNewConversation={() => onStartConversation(peer.desktopId)}
                          onReconnect={() => void run(peer.desktopId, () => connectRemotePeer(peer.desktopId))}
                          peer={peer}
                        />
                      ))}
                    </div>
                  )}
            </div>
          )}
      {unpairTarget
        ? (
      // `Dialog` rather than `ConfirmDeleteDialog`: the shared confirm
      // is worded for deletion ("Delete"), and unpairing a desktop is
      // not deleting anything the user created — the other machine
      // keeps all of its data.
            <Dialog
              description={t("unpairConfirmDesc", { name: peerBadgeText(unpairTarget, unpairTarget.desktopId) })}
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

function peerStatus(peer: RemotePeer, t: (key: string) => string): { tone: "accent" | "warning" | "danger"; label: string } {
  if (!peer.connected) {
    return { tone: peer.error ? "danger" : "warning", label: t("statusDisconnected") };
  }
  // A reachable bridge can still have an unreachable agent: the link is up and
  // every command fails. Saying "connected" there would be a lie the user
  // discovers only when their message does not send.
  if (!peer.agentAvailable) {
    return { tone: "warning", label: t("statusAgentUnavailable") };
  }
  return { tone: "accent", label: t("statusConnected") };
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
  const status = peerStatus(peer, t);
  // Starting a conversation needs the host's *agent*, not just its bridge: the
  // bridge can be reachable while every prompt fails. Offering the button there
  // would hand the user a box whose messages cannot go anywhere.
  const canConverse = peer.connected && peer.agentAvailable;

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
        {peer.error ? <div className="mt-1 text-xs text-danger">{peer.error}</div> : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        {canConverse
          ? (
              <Button disabled={busy} onClick={onNewConversation} size="sm" variant="primary">
                {t("newConversation")}
              </Button>
            )
          : null}
        {peer.connected
          ? <Button disabled={busy} onClick={onDisconnect} size="sm">{t("disconnect")}</Button>
          : <Button disabled={busy} onClick={onReconnect} size="sm" variant="primary">{t("connect")}</Button>}
        <Button disabled={busy} onClick={onEdit} size="sm" variant="ghost">{t("edit")}</Button>
      </div>
    </div>
  );
}
