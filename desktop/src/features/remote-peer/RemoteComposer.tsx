import type { RemotePeer } from "./remotePeerClient";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
import { abortRemoteRun, promptRemoteConversation } from "./remotePeerClient";

/**
 * The composer for a remote conversation.
 *
 * Deliberately plainer than the local one: no attachments, no slash commands,
 * no model picker in the box. Each of those is a *host-side* operation with its
 * own command and its own failure modes, and offering them here before they are
 * wired would produce buttons that look like they work and silently do nothing
 * — the failure mode users blame on the app rather than on the missing feature.
 *
 * It does carry the two controls that a conversation cannot be used without:
 * sending (with an empty `sessionId` for a conversation that does not exist yet)
 * and stopping a run that is in flight.
 */
export function RemoteComposer({
  desktopId,
  onCreated,
  onSent,
  peer,
  sessionId,
  streaming = false,
}: {
  desktopId: string;
  /**
   * Called only when the prompt *created* the conversation (the draft case),
   * with the ids the host chose. Opening the conversation is the caller's job:
   * the composer knows what to send, not what the user should be looking at.
   */
  onCreated?: (sessionId: string) => void;
  onSent: () => void;
  peer: RemotePeer | undefined;
  /** Empty means "a conversation that does not exist yet" — the host's own signal. */
  sessionId: string;
  streaming?: boolean;
}) {
  const { t } = useTranslation("remotePeer");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function send() {
    const message = text.trim();
    if (!message || busy)
      return;
    setBusy(true);
    setError(null);
    try {
      // The backend stamps the command id, and the host keys its durable
      // receipt on it, so a retry of a delivery cannot run the prompt twice.
      const ack = await promptRemoteConversation(desktopId, sessionId, message);
      // Cleared only after the host accepted it: a prompt that failed on the
      // wire must not cost the user the paragraph they wrote.
      setText("");
      if (!sessionId)
        onCreated?.(ack.sessionId);
      onSent();
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(false);
    }
  }

  async function stop() {
    setStopping(true);
    setError(null);
    try {
      await abortRemoteRun(desktopId, sessionId);
      onSent();
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setStopping(false);
    }
  }

  return (
    <div className="shrink-0 border-t border-line-soft px-4 py-3">
      {error ? <p className="mb-2 text-xs text-danger">{error}</p> : null}
      <div className="flex items-end gap-2">
        <TextInput
          aria-label={t("composerLabel")}
          onChange={event => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void send();
            }
          }}
          placeholder={t("composerPlaceholder", {
            name: peer?.name ?? peer?.desktopId ?? "",
          })}
          value={text}
        />
        {/* Stop replaces Send while a run is in flight rather than joining it:
            a conversation being answered on the host cannot take a second
            prompt, and the host would refuse one. The button is disabled while
            the abort is outstanding, so `disabled` is the single guard against
            a second abort — there is no second in-flight check to make. */}
        {streaming && !text.trim()
          ? (
              <Button disabled={stopping} onClick={() => void stop()} variant="secondary">
                {stopping ? t("stopping") : t("stop")}
              </Button>
            )
          : (
              <Button disabled={!text.trim() || busy} onClick={() => void send()} variant="primary">
                {busy ? t("sending") : t("send")}
              </Button>
            )}
      </div>
      <p className="mt-1.5 text-xs text-ink-muted">
        {t("runsOn", { name: peer?.name ?? peer?.desktopId ?? "" })}
      </p>
    </div>
  );
}
