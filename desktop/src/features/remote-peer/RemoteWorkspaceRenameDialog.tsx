import type { RemoteWorkspaceTarget } from "./mergeConversations";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { RenameDialog } from "../../components/layout/EntityDialogs";
import { renameRemoteWorkspace } from "./remotePeerClient";

/**
 * Rename a workspace on another machine.
 *
 * The shared dialog shell, not a new one: this app's own workspace rename is the
 * same box, and the only difference is where the name is written — the host's
 * store, through one command. What the words have to add is the machine, because
 * a name that changes on a heading the reader believes is theirs is the one
 * genuinely surprising thing here.
 *
 * No "generate title": a workspace has no conversation to name it from, and the
 * host publishes no such operation. Offering the button would be offering
 * something that cannot work.
 */
export function RemoteWorkspaceRenameDialog({
  onClose,
  onRenamed,
  target,
}: {
  onClose: () => void;
  /** Re-read the host's catalogue so the new heading shows. */
  onRenamed: () => void;
  target: RemoteWorkspaceTarget;
}) {
  const { t } = useTranslation("remotePeer");
  const [value, setValue] = useState(target.name);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    const name = value.trim();
    // The host refuses an empty name too; not sending one is the same answer
    // without the round trip, and the Save button is closed for it as well.
    if (!name || busy)
      return;
    setBusy(true);
    setError(null);
    try {
      await renameRemoteWorkspace(target.desktopId, target.workspaceId, name);
      onRenamed();
      onClose();
    }
    catch (err) {
      // Shown in the dialog: the host's refusal (an empty name, a name it does
      // not accept) is the reason the user needs, and closing would hide it.
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(false);
    }
  }

  return (
    <RenameDialog
      description={t("renameWorkspaceHint")}
      error={error}
      label={t("renameWorkspaceLabel")}
      onChange={setValue}
      onClose={onClose}
      onConfirm={() => void submit()}
      open
      submitting={busy}
      title={t("renameWorkspaceTitle", { name: target.name })}
      value={value}
    />
  );
}
