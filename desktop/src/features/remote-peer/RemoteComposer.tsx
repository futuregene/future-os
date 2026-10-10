import type { SkillCandidate } from "../../integrations/skills/skillsClient";
import type { RemotePeer, RemoteUpload } from "./remotePeerClient";
import { open } from "@tauri-apps/plugin-dialog";
import { Paperclip, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
import { errorMessage } from "../../lib/errors";
import { SkillRecommendCard } from "../agent/SkillRecommendCard";
import { abortRemoteRun, promptRemoteConversation, uploadRemoteFile } from "./remotePeerClient";

/**
 * What a skill recommendation holds the message for.
 *
 * The same contract this app's own composer uses (`SkillRecommendationProp`),
 * reduced to the four things a remote composer can act on.
 */
export interface RemoteSkillRecommendation {
  card: SkillCandidate | null;
  /** Resolve with the card to show (the send is held), or null to send normally. */
  onEvaluate: (draft: string) => Promise<SkillCandidate | null>;
  /** Install the skill on that host. True sends the message with `/name` appended. */
  onInstall: (card: SkillCandidate) => Promise<boolean>;
  onDismiss: () => void;
}

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
  skillRecommendation,
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
  /** Skill recommendation from that host, if the caller wired it. */
  skillRecommendation?: RemoteSkillRecommendation;
  streaming?: boolean;
}) {
  const { t } = useTranslation("remotePeer");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [installingSkill, setInstallingSkill] = useState(false);
  const [evaluating, setEvaluating] = useState(false);
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

  /**
   * Send one message, once nothing is holding the draft.
   *
   * `override` carries the text after a skill was installed and used
   * (`/name …`); the state update that follows is only so the field shows it.
   */
  async function sendNow(message: string, uploadsForSend: RemoteUpload[]) {
    setBusy(true);
    setError(null);
    try {
      // The backend stamps the command id, and the host keys its durable
      // receipt on it, so a retry of a delivery cannot run the prompt twice.
      const ack = await promptRemoteConversation(
        desktopId,
        sessionId,
        message,
        uploadsForSend.map(upload => upload.uploadId),
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

  async function send() {
    const message = text.trim();
    // An attachment with no words is still a message, which is how a file is
    // handed over without writing a prompt around it.
    if ((!message && uploads.length === 0) || busy || evaluating)
      return;

    // Skill recommendation, from *that* host: the draft is held while it is
    // asked, and a returned card keeps it unsubmitted until the user acts
    // (install, dismiss, or send again). A timeout, no match or a refusal sends
    // normally — recommendation is best-effort.
    const reco = skillRecommendation;
    if (reco && message) {
      // A card on screen and not yet acted on: a plain send means "send without
      // it", which is what the card's secondary button does. The send button
      // stays live in that state (only the recommend wait closes it), so
      // returning without sending would read as a broken button.
      if (reco.card) {
        reco.onDismiss();
        await sendNow(message, uploads);
        return;
      }
      setEvaluating(true);
      let card: SkillCandidate | null = null;
      try {
        card = await reco.onEvaluate(message);
      }
      finally {
        setEvaluating(false);
      }
      if (card)
        return;
    }

    await sendNow(message, uploads);
  }

  /**
   * Install the suggested skill on that host, then send the message with it
   * selected.
   *
   * A failed install leaves the card up and the draft untouched: the message was
   * written for a skill that is not there, and sending it anyway would be a
   * different message from the one the user agreed to.
   */
  async function installRecommended(reco: RemoteSkillRecommendation, card: SkillCandidate) {
    setInstallingSkill(true);
    setError(null);
    try {
      if (!await reco.onInstall(card))
        return;
      const withSkill = `${text.trim()} /${card.name} `;
      setText(withSkill);
      const staged = uploads;
      reco.onDismiss();
      await sendNow(withSkill.trim(), staged);
    }
    catch (err) {
      setError(errorMessage(err));
    }
    finally {
      setInstallingSkill(false);
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

  // Bound here rather than read inside the handlers: the card is only rendered
  // with one, so an install cannot be asked for without it — which is a fact
  // the types should carry rather than a guard that can never fire.
  const recommendationCard = skillRecommendation?.card ?? null;

  return (
    <div className="shrink-0 border-t border-line-soft px-4 py-3">
      {error ? <p className="mb-2 text-xs text-danger">{error}</p> : null}
      {/* The recommendation, above the box it is holding: it names a skill on
          that computer and offers to install it there. */}
      {skillRecommendation && recommendationCard
        ? (
            <SkillRecommendCard
              description={recommendationCard.description}
              installing={installingSkill}
              name={recommendationCard.name}
              onDismiss={() => {
                skillRecommendation.onDismiss();
                void sendNow(text.trim(), uploads);
              }}
              onInstall={() => void installRecommended(skillRecommendation, recommendationCard)}
            />
          )
        : null}
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
                disabled={(!text.trim() && uploads.length === 0) || busy || uploading || evaluating}
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
