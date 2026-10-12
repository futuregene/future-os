import type { ReactNode } from "react";
import type { SkillCandidate } from "../../integrations/skills/skillsClient";
import type { RemoteCommand } from "./remoteCommands";
import type { RemotePeer, RemoteUpload } from "./remotePeerClient";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowUp, Loader2, Paperclip, Square, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { readNativeClipboardFilePaths, savePastedFile, savePastedImage } from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { imageExtensionFromMime } from "../agent/attachments";
import { SkillRecommendCard } from "../agent/SkillRecommendCard";
import { RemoteApprovalMenu } from "./RemoteApprovalMenu";
import { buildRemoteSlashMenu, slashQuery, withoutSlashWord } from "./remoteCommands";
import { abortRemoteRun, promptRemoteConversation, uploadRemoteFile } from "./remotePeerClient";
import { RemoteSlashMenu } from "./RemoteSlashMenu";

/** How many files one paste may stage. The rest are reported, not silently dropped. */
const MAX_PASTED_FILES = 4;

/**
 * The field's ceiling, as the local editor's is: 40% of the window.
 *
 * A `<textarea>` does not grow with its content the way the local contenteditable
 * editor does, so the height is set from `scrollHeight` on every edit. The
 * ceiling is the sheet's own `max-h-[40vh]`; measuring in pixels here keeps one
 * number in one place (a computed style read would duplicate it).
 */
const MAX_FIELD_HEIGHT_RATIO = 0.4;

function growToContent(node: HTMLTextAreaElement): void {
  // `auto` first: with the current height still applied, `scrollHeight` would be
  // the *larger* of the content and the box, and a field never shrinks again.
  node.style.height = "auto";
  const ceiling = window.innerHeight * MAX_FIELD_HEIGHT_RATIO;
  node.style.height = `${Math.min(node.scrollHeight, ceiling)}px`;
}

/**
 * The commands this composer offers after `/`, built from what the host accepts.
 *
 * One command today, and it is here rather than in a module of its own because
 * this is its only consumer: a command whose host operation is unavailable is
 * left out rather than shown disabled, because a slash menu is a list of things
 * you can ask for.
 */
function useRemoteCommands({
  canCompact,
  onCompact,
}: {
  canCompact: boolean;
  onCompact: () => void;
}): RemoteCommand[] {
  const { t } = useTranslation("remotePeer");
  return canCompact
    ? [{
        description: t("compactHint"),
        id: "compact",
        label: t("compact"),
        run: onCompact,
      }]
    : [];
}

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
  canCompact = false,
  controls,
  desktopId,
  onCompact,
  onCreated,
  onSent,
  peer,
  sessionId,
  skillRecommendation,
  streaming = false,
  workspaceId = null,
}: {
  /**
   * Extra controls for the control row, before the machine hint and the send
   * button: the host's own model and thinking level, which this app's composer
   * also keeps beside the button that sends with them.
   */
  controls?: ReactNode;
  /**
   * Whether the host would accept a request to compact this conversation's
   * context: false for a draft (nothing to compact yet) and while a run is in
   * flight (a host answering one prompt at a time refuses one then).
   */
  canCompact?: boolean;
  desktopId: string;
  /** Ask the host to compact this conversation's context (the `/` menu's job). */
  onCompact?: () => void;
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
  /**
   * The workspace to create the conversation in, for a draft started from a
   * workspace heading. Null for a conversation with no workspace of its own.
   */
  workspaceId?: string | null;
}) {
  const { t } = useTranslation("remotePeer");
  const [text, setText] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
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
  async function pasteFiles(event: React.ClipboardEvent<HTMLTextAreaElement>) {
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
        // Names the workspace only while creating the conversation: one started
        // from a workspace heading belongs in that workspace, and the host
        // resolves the id against its own store.
        workspaceId,
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

  const commands = useRemoteCommands({
    canCompact: canCompact && !busy && !evaluating,
    onCompact: () => onCompact?.(),
  });
  const query = slashQuery(text);
  const slashGroups = query === null ? { commands: [] } : buildRemoteSlashMenu(query, commands);
  const [activeCommand, setActiveCommand] = useState(0);

  // A draft that arrived from somewhere other than a keystroke — the skill card's
  // insert, a cleared field — has to be measured too, or the box keeps the height
  // of the text it no longer holds.
  useEffect(() => {
    if (textareaRef.current)
      growToContent(textareaRef.current);
  }, [text]);

  /** Ask the host for one command, then take its word out of the draft. */
  function chooseCommand(command: RemoteCommand) {
    setText(withoutSlashWord(text));
    setActiveCommand(0);
    command.run();
  }

  /**
   * What Enter means.
   *
   * One function for both paths that can produce it — the field's own handler and
   * the form's submit — because they disagreed: the form knew about the slash
   * menu and the field did not, so Enter while the menu was open sent the draft
   * (including the word that opened it) instead of running the command.
   */
  function submit() {
    const highlighted = slashGroups.commands[activeCommand];
    if (highlighted) {
      chooseCommand(highlighted);
      return;
    }
    void send();
  }

  return (
    // The same shell as this app's own composer (`Composer.tsx`), at the same
    // size and in the same place: `mx-auto max-w-3xl` in a `px-8 pb-5` band at
    // the foot of the conversation. The width is the part that matters — the
    // message column above it is `max-w-3xl`, and a composer wider than the text
    // it sends puts the caret somewhere other than where the sentence is.
    <div className="shrink-0 px-8 pb-5 pt-2">
      <form
        className="relative mx-auto w-full max-w-3xl rounded-lg border border-line bg-surface/95 p-2 shadow-panel backdrop-blur"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        {query !== null
          ? (
              <RemoteSlashMenu
                activeIndex={activeCommand}
                groups={slashGroups}
                onChoose={chooseCommand}
              />
            )
          : null}
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
          {/* The field is the local composer's, wrapped around a plain textarea.
              The local editor is a contenteditable div — it has the `@`, `/` and
              `#` menus built into it — so the markup cannot be shared; what can
              be shared is everything a reader sees: no inner border (the box's
              own border is the shell's), the same padding and type, and the same
              growth up to 40% of the window. A bordered `TextInput` inside a
              bordered shell put a control inside a control and made the two
              composers look like two different products. */}
          <textarea
            aria-label={t("composerLabel")}
            className="max-h-[40vh] min-h-14 w-full resize-none overflow-y-auto whitespace-pre-wrap wrap-break-word bg-transparent px-2 py-1 text-sm leading-5 text-ink outline-none placeholder:text-ink-muted"
            onChange={(event) => {
              setText(event.target.value);
              growToContent(event.currentTarget);
            }}
            onPaste={event => void pasteFiles(event)}
            onKeyDown={(event) => {
              // The slash menu owns the arrows and Enter while it is open: the
              // word being typed is a command, not a sentence.
              if (query !== null && slashGroups.commands.length > 0) {
                if (event.key === "ArrowDown") {
                  event.preventDefault();
                  setActiveCommand(index => (index + 1) % slashGroups.commands.length);
                  return;
                }
                if (event.key === "ArrowUp") {
                  event.preventDefault();
                  setActiveCommand(index => (index - 1 + slashGroups.commands.length) % slashGroups.commands.length);
                  return;
                }
                if (event.key === "Escape") {
                  event.preventDefault();
                  setText(withoutSlashWord(text));
                  return;
                }
              }
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                submit();
              }
            }}
            placeholder={t("composerPlaceholder", {
              name: peer?.name ?? peer?.desktopId ?? "",
            })}
            ref={textareaRef}
            rows={1}
            value={text}
          />
          {/* The control row, in this app's own arrangement: attachment and the
            host's approval mode at the left, the host's model and thinking level
            beside the send button at the right. The same slots in the same order
            as the local composer, because these are the same four decisions —
            what goes with the message, how much the host may do on its own, and
            which model reasons about it. */}
          <div className="flex flex-wrap items-center justify-between gap-y-1 pt-1">
            <div className="flex min-w-0 items-center gap-1">
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
              <RemoteApprovalMenu desktopId={desktopId} peer={peer} />
            </div>
            <div className="ms-auto flex min-w-0 items-center gap-2">
              {controls}
              {/* Stop replaces Send while a run is in flight rather than joining it:
                a conversation being answered on the host cannot take a second
                prompt, and the host would refuse one. The button is disabled while
                the abort is outstanding, so `disabled` is the single guard against
                a second abort — there is no second in-flight check to make.
                Icon buttons, at the local composer's size and colour: the same
                two controls in the same place on both screens. */}
              {streaming && !text.trim() && uploads.length === 0
                ? (
                    <button
                      aria-label={stopping ? t("stopping") : t("stop")}
                      className="inline-flex size-7 items-center justify-center rounded-md bg-ink text-surface transition-colors hover:bg-ink-soft disabled:cursor-not-allowed disabled:opacity-50"
                      disabled={stopping}
                      onClick={() => void stop()}
                      title={stopping ? t("stopping") : t("stop")}
                      type="button"
                    >
                      {stopping ? <Loader2 className="size-3.5 animate-spin" /> : <Square className="size-3 fill-current" />}
                    </button>
                  )
                : (
                    <button
                      aria-label={busy ? t("sending") : t("send")}
                      className="inline-flex size-7 items-center justify-center rounded-md bg-accent text-white transition-colors hover:bg-accent-hover disabled:bg-accent-disabled"
                      disabled={(!text.trim() && uploads.length === 0) || busy || uploading || evaluating}
                      title={busy ? t("sending") : t("send")}
                      type="submit"
                    >
                      {busy ? <Loader2 className="size-3.5 animate-spin" /> : <ArrowUp className="size-3.5" />}
                    </button>
                  )}
            </div>
          </div>
        </div>
      </form>
    </div>
  );
}
