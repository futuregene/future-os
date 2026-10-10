import type { RemotePeer, RemoteUpload } from "./remotePeerClient";
import { open } from "@tauri-apps/plugin-dialog";
import { Paperclip, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
import { errorMessage } from "../../lib/errors";
import { abortRemoteRun, promptRemoteConversation, uploadRemoteFile } from "./remotePeerClient";

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
  /**
   * Files already staged on the host, waiting to be attached to a message.
   *
   * Held here rather than in the parent because they are not part of the
   * conversation until they are sent: abandoning the composer abandons them,
   * and the host discards an unreferenced upload by itself.
   */
  const [uploads, setUploads] = useState<RemoteUpload[]>([]);
  const [uploading, setUploading] = useState(false);

  /**
   * Stage a file on the host.
   *
   * The bytes travel at *attach* time rather than at send time so a large upload
   * is not made to share a click with the prompt: the user can see it land, and a
   * failed transfer leaves their message intact.
   *
   * No in-flight guard: the control that calls this is disabled while an upload
   * is outstanding, so a second call cannot be produced, and a guard no test can
   * reach is a guard no test can trust.
   */
  async function attach() {
    setUploading(true);
    setError(null);
    try {
      const picked = await open({ multiple: false });
      const path = typeof picked === "string" ? picked : null;
      if (!path)
        return;
      const staged = await uploadRemoteFile({ desktopId, path });
      setUploads(current => [...current, staged]);
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setUploading(false);
    }
  }

  async function send() {
    const message = text.trim();
    // An attachment with no words is still a message, which is how a file is
    // handed over without writing a prompt around it.
    if ((!message && uploads.length === 0) || busy)
      return;
    setBusy(true);
    setError(null);
    try {
      // The backend stamps the command id, and the host keys its durable
      // receipt on it, so a retry of a delivery cannot run the prompt twice.
      const ack = await promptRemoteConversation(
        desktopId,
        sessionId,
        message,
        uploads.map(upload => upload.uploadId),
      );
      // Cleared only after the host accepted it: a prompt that failed on the
      // wire must not cost the user the paragraph they wrote or the file they
      // staged.
      setText("");
      setUploads([]);
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
      {/* What is about to be sent, named, with a way back out: an attachment the
          user cannot see is one they cannot remove. */}
      {uploads.length > 0
        ? (
            <div className="mb-2 flex flex-wrap gap-1.5">
              {uploads.map(upload => (
                <span
                  className="inline-flex items-center gap-1 rounded-md border border-line-soft bg-surface-subtle px-2 py-0.5 text-xs text-ink-soft"
                  key={upload.uploadId}
                >
                  <Paperclip aria-hidden className="size-3" />
                  <span className="max-w-48 truncate">{upload.name}</span>
                  <button
                    aria-label={t("removeAttachment", { name: upload.name })}
                    className="text-ink-muted hover:text-ink"
                    onClick={() => setUploads(current => current.filter(item => item.uploadId !== upload.uploadId))}
                    type="button"
                  >
                    <X className="size-3" />
                  </button>
                </span>
              ))}
            </div>
          )
        : null}
      <div className="flex items-end gap-2">
        <Button disabled={uploading} onClick={() => void attach()} size="sm" variant="ghost">
          <Paperclip className="size-3.5" />
          {uploading ? t("attaching") : t("attach")}
        </Button>
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
        {streaming && !text.trim() && uploads.length === 0
          ? (
              <Button disabled={stopping} onClick={() => void stop()} variant="secondary">
                {stopping ? t("stopping") : t("stop")}
              </Button>
            )
          : (
              <Button
                disabled={(!text.trim() && uploads.length === 0) || busy || uploading}
                onClick={() => void send()}
                variant="primary"
              >
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
