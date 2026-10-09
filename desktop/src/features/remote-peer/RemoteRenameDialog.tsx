import type { MergedConversation } from "./mergeConversations";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { TextInput } from "../../components/ui/TextInput";
import { renameRemoteConversation } from "./remotePeerClient";

/**
 * Rename a remote conversation.
 *
 * Its own dialog rather than the local one: that one carries the local store's
 * generating/error state and a "generate title" action backed by the local
 * agent. Pointing it at a remote session would mean faking a local thread and
 * leaving a button that asks the wrong machine to name a conversation it has
 * never seen.
 */
export function RemoteRenameDialog({
  conversation,
  onClose,
  onRenamed,
}: {
  conversation: MergedConversation;
  onClose: () => void;
  /** Re-read the owning host's catalogue so the new title shows. */
  onRenamed: () => void;
}) {
  const { t } = useTranslation("remotePeer");
  const [value, setValue] = useState(conversation.title);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const desktopId = conversation.desktopId;

  async function submit() {
    const name = value.trim();
    if (!desktopId || !name || busy)
      return;
    setBusy(true);
    setError(null);
    try {
      await renameRemoteConversation(
        desktopId,
        { sessionId: conversation.id, threadId: conversation.threadId },
        name,
      );
      onRenamed();
      onClose();
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      description={t("renameHint")}
      footer={(
        <>
          <Button onClick={onClose} variant="secondary">{t("cancel")}</Button>
          <Button disabled={!value.trim() || busy} onClick={() => void submit()} variant="primary">
            {busy ? t("saving") : t("save")}
          </Button>
        </>
      )}
      onClose={onClose}
      open
      title={t("renameTitle")}
    >
      {error ? <p className="mb-2 text-xs text-danger">{error}</p> : null}
      <TextInput
        aria-label={t("renameTitle")}
        onChange={event => setValue(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            void submit();
          }
        }}
        value={value}
      />
    </Dialog>
  );
}
