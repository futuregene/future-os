import type { MergedConversation } from "./mergeConversations";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { TextInput } from "../../components/ui/TextInput";
import { getLanguage } from "../../i18n";
import { generateRemoteTitle, renameRemoteConversation } from "./remotePeerClient";

/**
 * Rename a remote conversation.
 *
 * Its own dialog rather than the local one: that one carries the local store's
 * generating/error state. The "generate title" action is here now too, but
 * pointed at the host — the local one would ask the wrong machine to name a
 * conversation it has never seen.
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
  const { t: tLayout } = useTranslation("layout");
  const [value, setValue] = useState(conversation.title);
  const [busy, setBusy] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const desktopId = conversation.desktopId;
  const sessionId = conversation.id;

  async function submit() {
    const name = value.trim();
    if (!desktopId || !name || busy || generating)
      return;
    setBusy(true);
    setError(null);
    try {
      await renameRemoteConversation(
        desktopId,
        { sessionId, threadId: conversation.threadId },
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

  /**
   * Fill the field with a title the host produced.
   *
   * It is not saved: the local dialog behaves the same way, and a generated
   * title usually needs a word changed before it is worth keeping.
   */
  async function generate() {
    if (!desktopId || busy || generating)
      return;
    setGenerating(true);
    setError(null);
    try {
      setValue(await generateRemoteTitle(desktopId, sessionId, getLanguage()));
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setGenerating(false);
    }
  }

  return (
    <Dialog
      description={t("renameHint")}
      footer={(
        <>
          <Button onClick={onClose} variant="secondary">{t("cancel")}</Button>
          <Button disabled={!value.trim() || busy || generating} onClick={() => void submit()} variant="primary">
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
      {/* The same action, wording and hint as this app's own rename dialog: the
          host names the conversation with its own model, and the result lands in
          the field to be reviewed before saving. */}
      <div className="mt-3 space-y-2">
        <Button
          disabled={busy || generating}
          onClick={() => void generate()}
          type="button"
          variant="secondary"
        >
          {tLayout(generating ? "appShellDialogs.generatingTitle" : "appShellDialogs.generateTitle")}
        </Button>
        <p className="text-xs text-ink-muted">{tLayout("appShellDialogs.generateTitleHint")}</p>
      </div>
    </Dialog>
  );
}
