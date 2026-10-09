import type { RemotePeer } from "./remotePeerClient";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
import { requestRemotePeer } from "./remotePeerClient";

/**
 * The composer for a remote conversation.
 *
 * Deliberately plainer than the local one: no attachments, no slash commands,
 * no model picker in the box. Each of those is a *host-side* operation with its
 * own command and its own failure modes, and offering them here before they are
 * wired would produce buttons that look like they work and silently do nothing
 * — the failure mode users blame on the app rather than on the missing feature.
 *
 * The prompt command is the phone's, so its guarantees carry over: the host
 * answers with a durable receipt keyed by the command id, which is what makes a
 * retry safe.
 */
export function RemoteComposer({
  desktopId,
  onSent,
  peer,
  sessionId,
}: {
  desktopId: string;
  onSent: () => void;
  peer: RemotePeer | undefined;
  sessionId: string;
}) {
  const { t } = useTranslation("remotePeer");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function send() {
    const message = text.trim();
    if (!message || busy)
      return;
    setBusy(true);
    setError(null);
    try {
      // A stable id per attempt: the host keys its receipt on it, so a retry of
      // the same id cannot run the prompt twice.
      const id = `cmd_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
      await requestRemotePeer(desktopId, {
        type: "prompt",
        sessionId,
        message,
        id,
      }, sessionId);
      // Cleared only after the host accepted it: a prompt that failed on the
      // wire must not cost the user the paragraph they wrote.
      setText("");
      onSent();
    }
    catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
    finally {
      setBusy(false);
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
        <Button disabled={!text.trim() || busy} onClick={() => void send()} variant="primary">
          {busy ? t("sending") : t("send")}
        </Button>
      </div>
      <p className="mt-1.5 text-xs text-ink-muted">
        {t("runsOn", { name: peer?.name ?? peer?.desktopId ?? "" })}
      </p>
    </div>
  );
}
