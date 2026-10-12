import type { SkillCandidate } from "../../integrations/skills/skillsClient";
import type { RemotePeer, RemoteUpload } from "./remotePeerClient";
import { open } from "@tauri-apps/plugin-dialog";
import { Paperclip, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TextInput } from "../../components/ui/TextInput";
import { readNativeClipboardFilePaths, savePastedFile, savePastedImage } from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { imageExtensionFromMime } from "../agent/attachments";
import { SkillRecommendCard } from "../agent/SkillRecommendCard";
import { abortRemoteRun, promptRemoteConversation, uploadRemoteFile } from "./remotePeerClient";

/** How many files one paste may stage. The rest are reported, not silently dropped. */
const MAX_PASTED_FILES = 4;

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
   * Stage whatever the clipboard carried.
   *
   * The same sources the local composer reads — a Finder copy arrives as opaque
   * `File`s whose original paths have to be recovered, a screenshot as image
   * bytes — and the difference is only where the bytes end up: on the *host*,
   * because that is the machine that will read the file. Text pastes are left to
   * the field.
   */
  async function pasteFiles(event: React.ClipboardEvent<HTMLInputElement>) {
    const data = event.clipboardData;
    if (!data || busy || uploading)
      return;
    const files = [...data.files];
    const hasUriList = data.types.includes("text/uri-list") && !!data.getData("text/uri-list");
    if (files.length === 0 && !hasUriList)
      return;
    // Recovered paths are used when the platform hid them (WKWebView turns a
    // Finder copy into `File`s with no `path`), and image bytes are written to a
    // temporary file first because the host's upload command takes a path.
    //
    // The recovered paths win outright rather than being added to the `File`s
    // that produced them: those `File`s are the *same* files, so processing them
    // too would stage every copied file twice — once by path, once by bytes. The
    // local composer takes the same shortcut for the same reason.
    const nativePaths = await readNativeClipboardFilePaths().catch(() => []);
    const paths = [...nativePaths];
    if (nativePaths.length === 0) {
      for (const file of files) {
        if (paths.length >= MAX_PASTED_FILES)
          break;
        try {
          const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
          const saved = file.type.startsWith("image/")
            ? await savePastedImage({ bytes, extension: imageExtensionFromMime(file.type) ?? "png" })
            : await savePastedFile({ bytes, name: file.name });
          paths.push(saved.path);
        }
        catch {
          setError(t("pasteUnreadable", { name: file.name }));
          return;
        }
      }
    }
    if (paths.length === 0)
      return;
    // Only now, with something to send: a paste of text or a link must still
    // reach the field it was aimed at.
    event.preventDefault();
    // The count is decided before the uploads start, and the message is set with
    // the reset rather than before it: setting it and then calling
    // `setError(null)` wiped the warning in the same tick, so a paste of ten
    // files staged four of them and said nothing at all about the other six.
    const staged = paths.slice(0, MAX_PASTED_FILES);
    const overLimit = paths.length > MAX_PASTED_FILES;
    setUploading(true);
    setError(overLimit ? t("pasteTooMany", { max: MAX_PASTED_FILES }) : null);
    try {
      const uploads: RemoteUpload[] = [];
      for (const path of staged)
        uploads.push(await uploadRemoteFile({ desktopId, path }));
      setUploads(current => [...current, ...uploads]);
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
    // The same shell as this app's own composer (`Composer.tsx`): a bordered,
    // floating panel with the attachment chips above the field and the controls
    // in a row underneath. One machine's composer looking like a plain form
    // while the other's is a panel makes switching between them read as two
    // different apps — the conversation is the same thing either way.
    <div className="shrink-0 px-4 pb-4 pt-2">
      <form
        className="relative rounded-lg border border-line bg-surface/95 p-2 shadow-panel backdrop-blur"
        onSubmit={(event) => {
          event.preventDefault();
          void send();
        }}
      >
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
              <div className="mb-2 flex flex-wrap items-center gap-1.5 px-1">
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
        <div className="flex flex-col gap-1 px-1 pt-1">
          <TextInput
            aria-label={t("composerLabel")}
            onChange={event => setText(event.target.value)}
            onPaste={event => void pasteFiles(event)}
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
          {/* Attach sits at the left of the controls row and the send control at
            the right, which is where this app's own composer puts them. */}
          <div className="flex items-center justify-between gap-1">
            <button
              aria-label={t("attach")}
              className="inline-flex size-7 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
              disabled={uploading}
              onClick={() => void attach()}
              title={t("attach")}
              type="button"
            >
              <Paperclip className="size-3.5" />
            </button>
            <div className="flex items-center gap-2">
              {/* Where the prompt will run. In the controls row rather than under
                the whole panel: it qualifies the send button beside it. */}
              <span className="truncate text-xs text-ink-muted">
                {uploading ? t("attaching") : t("runsOn", { name: peer?.name ?? peer?.desktopId ?? "" })}
              </span>
              {/* Stop replaces Send while a run is in flight rather than joining it:
                a conversation being answered on the host cannot take a second
                prompt, and the host would refuse one. The button is disabled while
                the abort is outstanding, so `disabled` is the single guard against
                a second abort — there is no second in-flight check to make. */}
              {streaming && !text.trim() && uploads.length === 0
                ? (
                    <Button disabled={stopping} onClick={() => void stop()} size="sm" variant="secondary">
                      {stopping ? t("stopping") : t("stop")}
                    </Button>
                  )
                : (
                    <Button
                      disabled={(!text.trim() && uploads.length === 0) || busy || uploading || evaluating}
                      size="sm"
                      type="submit"
                      variant="primary"
                    >
                      {busy ? t("sending") : t("send")}
                    </Button>
                  )}
            </div>
          </div>
        </div>
      </form>
    </div>
  );
}
