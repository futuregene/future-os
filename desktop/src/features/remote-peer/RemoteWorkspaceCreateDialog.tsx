import type { RemotePeer, RemoteWorkspaceRow } from "./remotePeerClient";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { TextInput } from "../../components/ui/TextInput";
import { peerBadgeText } from "./peerIcons";
import { createRemoteWorkspace } from "./remotePeerClient";

/**
 * Create a workspace on another machine, after the device step named it.
 *
 * The path is typed rather than picked: a folder picker here browses *this*
 * computer, while the workspace is a directory on that one — the same reason
 * the host's own workspace settings page takes a typed path. The host keeps its
 * own rules (the directory must already exist, an existing path reopens that
 * row), so a refusal is shown here rather than second-guessed.
 */
export function RemoteWorkspaceCreateDialog({
  onClose,
  onCreated,
  peer,
}: {
  onClose: () => void;
  /** The new workspace is on `peer`; the caller refreshes that host's catalogue. */
  onCreated: (workspace: RemoteWorkspaceRow) => void;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const { t: tCommon } = useTranslation("common");
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const host = peerBadgeText(peer, peer.desktopId);

  async function submit() {
    const trimmed = path.trim();
    // The host refuses an empty path too; not sending one is the same answer
    // without the round trip.
    if (!trimmed || busy)
      return;
    setBusy(true);
    setError(null);
    try {
      onCreated(await createRemoteWorkspace(peer.desktopId, trimmed, name));
    }
    catch (caught) {
      // Shown in the dialog rather than closing it: the host's reason (a path
      // that does not exist there, a name it does not accept) is what the user
      // needs to correct.
      setError(caught instanceof Error ? caught.message : String(caught));
    }
    finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      description={t("workspacesAddHint")}
      footer={(
        <>
          <Button onClick={onClose} size="sm" variant="ghost">{tCommon("cancel")}</Button>
          <Button
            disabled={busy || !path.trim()}
            onClick={() => void submit()}
            size="sm"
            variant="primary"
          >
            {t("workspacesCreateAction")}
          </Button>
        </>
      )}
      onClose={onClose}
      open
      title={t("newWorkspaceOn", { host })}
    >
      <div className="space-y-3">
        <label className="block text-xs text-ink-muted">
          {t("workspacesPathLabel")}
          <TextInput
            aria-label={t("workspacesPathLabel")}
            autoFocus
            disabled={busy}
            onChange={event => setPath(event.target.value)}
            placeholder={t("workspacesPathPlaceholder")}
            value={path}
          />
        </label>
        <label className="block text-xs text-ink-muted">
          {t("workspacesNameLabel")}
          <TextInput
            aria-label={t("workspacesNameLabel")}
            disabled={busy}
            onChange={event => setName(event.target.value)}
            value={name}
          />
        </label>
        {error ? <p className="text-xs text-danger">{error}</p> : null}
      </div>
    </Dialog>
  );
}
