import type { RemotePeer, RemoteWorkspaceRow } from "./remotePeerClient";
import { Pin, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
import { SettingsList, SettingsRow, SettingsSection } from "../settings/SettingsPrimitives";
import {
  createRemoteWorkspace,
  deleteRemoteWorkspace,
  listRemoteWorkspaces,
  setRemoteWorkspacePinned,
} from "./remotePeerClient";

/**
 * Another computer's workspaces.
 *
 * A workspace is a directory on that machine, so this is where its folders are
 * named and ordered — not a way to reach into them. Creating one registers a
 * path that must already exist there, which is why the path is typed rather
 * than picked: this machine has no folder picker for another machine's disk.
 *
 * Removing one takes that workspace's conversations with it, so it confirms
 * first and says what it is about to do.
 */
export function RemoteWorkspacesPanel({
  available,
  onChanged,
  peer,
}: {
  available: boolean;
  /** A workspace change can rename or remove conversations, so the host is re-read. */
  onChanged: () => void;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const [workspaces, setWorkspaces] = useState<RemoteWorkspaceRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [path, setPath] = useState("");
  const [name, setName] = useState("");
  const [removing, setRemoving] = useState<RemoteWorkspaceRow | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setWorkspaces(await listRemoteWorkspaces(peer.desktopId));
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }, [peer.desktopId]);

  useEffect(() => {
    void load();
  }, [load]);

  async function run(action: () => Promise<void>): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      await action();
      // A rename or removal changes the conversation list too, so it is re-read
      // rather than patched here.
      onChanged();
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
    finally {
      setBusy(false);
    }
  }

  async function add() {
    // The button is closed without a path, so this only runs with one.
    await run(async () => {
      await createRemoteWorkspace(peer.desktopId, path, name);
      setPath("");
      setName("");
      setWorkspaces(await listRemoteWorkspaces(peer.desktopId));
    });
  }

  const closed = !available || busy;

  return (
    <SettingsSection description={t("workspacesHint")} title={t("workspacesSection")}>
      {error ? <p className="text-xs text-danger">{error}</p> : null}

      <SettingsList>
        {workspaces.length === 0
          ? <SettingsRow title={t("workspacesEmpty")} />
          : workspaces.map(workspace => (
              <SettingsRow
                description={workspace.path}
                key={workspace.id}
                title={(
                  <span className="flex items-center gap-1.5">
                    {workspace.pinned
                      ? <Pin aria-hidden className="size-3 shrink-0 text-accent" />
                      : null}
                    {workspace.name}
                  </span>
                )}
              >
                <div className="flex items-center gap-2">
                  <Button
                    disabled={closed}
                    onClick={() => void run(async () => {
                      await setRemoteWorkspacePinned(peer.desktopId, workspace.id, !workspace.pinned);
                      setWorkspaces(await listRemoteWorkspaces(peer.desktopId));
                    })}
                    size="sm"
                    variant="ghost"
                  >
                    {workspace.pinned ? t("workspacesUnpin") : t("workspacesPin")}
                  </Button>
                  <Button
                    disabled={closed}
                    onClick={() => setRemoving(workspace)}
                    size="sm"
                    variant="danger-soft"
                  >
                    <Trash2 aria-hidden className="size-3.5" />
                  </Button>
                </div>
              </SettingsRow>
            ))}
      </SettingsList>

      <div className="mt-2 flex items-end gap-2">
        <label className="min-w-0 flex-1 text-xs text-ink-muted">
          {t("workspacesPathLabel")}
          <TextInput
            aria-label={t("workspacesPathLabel")}
            disabled={closed}
            onChange={event => setPath(event.target.value)}
            placeholder={t("workspacesPathPlaceholder")}
            value={path}
          />
        </label>
        <label className="w-40 text-xs text-ink-muted">
          {t("workspacesNameLabel")}
          <TextInput
            aria-label={t("workspacesNameLabel")}
            disabled={closed}
            onChange={event => setName(event.target.value)}
            value={name}
          />
        </label>
        <Button disabled={closed || !path.trim()} onClick={() => void add()} size="sm">
          {t("workspacesAdd")}
        </Button>
      </div>
      <p className="text-xs leading-5 text-ink-muted">{t("workspacesAddHint")}</p>

      {removing
        ? (
            <div className="flex items-center justify-between gap-3 rounded-lg border border-line-soft bg-surface px-4 py-3">
              <span className="text-sm text-ink">
                {t("workspacesRemoveConfirm", { host: peer.name || peer.desktopId, workspace: removing.name })}
              </span>
              <div className="flex items-center gap-2">
                <Button onClick={() => setRemoving(null)} size="sm" variant="ghost">
                  {t("cancel")}
                </Button>
                <Button
                  disabled={busy}
                  onClick={() => void run(async () => {
                    await deleteRemoteWorkspace(peer.desktopId, removing.id);
                    setRemoving(null);
                    setWorkspaces(await listRemoteWorkspaces(peer.desktopId));
                  })}
                  size="sm"
                  variant="danger-soft"
                >
                  {t("workspacesRemove")}
                </Button>
              </div>
            </div>
          )
        : null}
    </SettingsSection>
  );
}
