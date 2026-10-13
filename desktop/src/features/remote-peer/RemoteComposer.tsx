import type { ReactNode } from "react";
import type { SkillCandidate } from "../../integrations/skills/skillsClient";
import type { RemoteCommand, RemoteSkillChoice, RemoteSlashItem } from "./remoteCommands";
import type { RemotePeer, RemoteSkillInfo, RemoteUpload } from "./remotePeerClient";
import type { PendingPrompt } from "./remotePromptOutbox";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowUp, Loader2, Paperclip, Square, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { readNativeClipboardFilePaths, savePastedFile, savePastedImage } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { errorMessage } from "../../lib/errors";
import { imageExtensionFromMime, splitFileName } from "../agent/attachments";
import { SkillRecommendCard } from "../agent/SkillRecommendCard";
import { RemoteApprovalMenu } from "./RemoteApprovalMenu";
import { buildRemoteSlashMenu, insertRemoteSkill, remoteSlashItems, slashQuery, withoutSlashWord } from "./remoteCommands";
import { clearRemoteComposerDraft, loadRemoteComposerDraft, saveRemoteComposerDraft } from "./remoteComposerDraft";
import { abortRemoteRun, listRemoteInstalledSkills, uploadRemoteFile } from "./remotePeerClient";
import { getRemoteOutbox, isRecoverableSendError, sendRemotePrompt } from "./remotePromptOutbox";
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
 * The skills the host has installed, for the `/` menu.
 *
 * Read while the menu is open rather than once at mount, for the reason the
 * phone reloads its picker on every open: the list belongs to the other machine,
 * and a skill installed there a minute ago belongs in the menu now. What was
 * read last stays on screen while the fresh read is in flight, so the menu does
 * not flicker empty between keystrokes.
 *
 * `enabled` is the caller's answer to "could this host have skills to offer" —
 * the capability it declared, and whether a menu is open at all — so a host that
 * predates the feature is never asked and a closed menu never costs a request.
 */
function useRemoteSkills({
  desktopId,
  enabled,
}: {
  desktopId: string;
  enabled: boolean;
}): RemoteSkillChoice[] {
  const { i18n } = useTranslation("remotePeer");
  const [skills, setSkills] = useState<RemoteSkillInfo[]>([]);

  useEffect(() => {
    if (!enabled)
      return;
    let cancelled = false;
    void listRemoteInstalledSkills(desktopId)
      .then((rows) => {
        if (!cancelled)
          setSkills(rows);
      })
      .catch(() => {
        // A host that will not answer has no skills to offer this time. The menu
        // still shows what it has (the commands), which is better than an error
        // about a list the user never asked to see.
      });
    return () => {
      cancelled = true;
    };
  }, [desktopId, enabled]);

  // The name is the host's invocation name — the one `/` has to name — while the
  // description follows this app's language, and the other language's is kept
  // for the search. That is the phone's own rule (its row prints the English
  // command with a localized description), and the local composer's.
  const chinese = i18n.language.startsWith("zh");
  return useMemo(
    () => skills.map(skill => ({
      name: skill.name,
      description: chinese
        ? skill.descriptionZh || skill.description
        : skill.description || skill.descriptionZh || "",
      descriptionZh: skill.descriptionZh ?? null,
    })),
    [chinese, skills],
  );
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
 * Deliberately plainer than the local one: no `@` file mentions (a file is
 * attached with the paperclip, and the host receives it by id) and no `#`
 * conversation references. What the two share is what a reader compares:
 * the same shell, the same control row, the same `/` menu — the host's commands
 * and its installed skills, which is what makes the placeholder read like this
 * app's own.
 *
 * It does carry the two controls that a conversation cannot be used without:
 * sending (with an empty `sessionId` for a conversation that does not exist yet)
 * and stopping a run that is in flight.
 */
export function RemoteComposer({
  canCompact = false,
  className,
  controls,
  desktopId,
  freshModelId = null,
  freshThinkingLevel = null,
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
   * Layout from the caller, as this app's own composer takes it.
   *
   * The conversation frame decides where the composer sits — the band over the
   * transcript's foot owns the width and the padding — so the composer must not
   * carry its own margins, or the two frames drift apart again the moment one of
   * them moves.
   */
  className?: string;
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
  /**
   * The model and thinking level the picker is showing for a *draft*.
   *
   * Sent with the first prompt so the conversation is created with what the
   * reader saw: it does not exist on the host yet, so there is nothing there to
   * set beforehand, and the values cannot be applied afterwards without a second
   * round trip that races the run.
   */
  freshModelId?: string | null;
  freshThinkingLevel?: string | null;
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
   * A prompt that was queued because the host was unreachable. The UI shows the
   * user that something is waiting to be sent.
   */
  const [pendingPrompt, setPendingPrompt] = useState<PendingPrompt | null>(null);

  // Load the draft for this host+conversation on mount / when the conversation
  // changes, and persist edits as they happen. The key includes desktopId so
  // the same session id on two different hosts does not share a draft.
  useEffect(() => {
    const draft = loadRemoteComposerDraft(desktopId, sessionId);
    setText(draft?.text ?? "");
    setUploads(draft?.uploads ?? []);
    const outbox = getRemoteOutbox(desktopId, sessionId);
    setPendingPrompt(outbox.prompt);
  }, [desktopId, sessionId]);

  useEffect(() => {
    saveRemoteComposerDraft(desktopId, sessionId, { text, uploads });
  }, [desktopId, sessionId, text, uploads]);

  // When the link comes back, try to flush any queued prompt/continuation.
  useEffect(() => {
    if (!peer?.connected)
      return;
    let active = true;
    import("./remotePromptOutbox").then(({ recoverRemoteOutbox, getRemoteOutbox }): void => {
      if (!active)
        return;
      void recoverRemoteOutbox(desktopId, sessionId).then(() => {
        const outbox = getRemoteOutbox(desktopId, sessionId);
        if (active)
          setPendingPrompt(outbox.prompt);
      });
    });
    return () => {
      active = false;
    };
  }, [peer?.connected, desktopId, sessionId]);

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
      // The outbox generates a stable command id, persists the prompt, and
      // clears it once the host accepts. If the prompt is queued because the
      // link is down, the outbox leaves it for recovery.
      const ack = await sendRemotePrompt(desktopId, sessionId, {
        text: message,
        attachments: uploadsForSend,
        workspaceId,
        freshModelId,
        freshThinkingLevel,
      });
      // Cleared only after the host accepted it: a prompt that failed on the
      // wire must not cost the user the paragraph they wrote or the file they
      // staged.
      setText("");
      setUploads([]);
      clearRemoteComposerDraft(desktopId, sessionId);
      setPendingPrompt(null);
      if (!sessionId)
        onCreated?.(ack.sessionId);
      onSent();
    }
    catch (err) {
      if (isRecoverableSendError(err)) {
        const outbox = getRemoteOutbox(desktopId, sessionId);
        setPendingPrompt(outbox.prompt);
        setError(t("promptQueued"));
      }
      else {
        setError(err instanceof Error ? err.message : String(err));
      }
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
  /**
   * Whether this host could have skills to put in the menu.
   *
   * The capability it declared, not a guess: a host that never announced
   * `skills_v1` is a host whose `/` menu is commands only, and one that never had
   * the feature is never asked for a list it cannot send.
   */
  const supportsSkills = peer?.features.includes("skills_v1") === true;
  const hostSkills = useRemoteSkills({ desktopId, enabled: query !== null && supportsSkills });
  const slashGroups = query === null ? { commands: [], skills: [] } : buildRemoteSlashMenu(query, commands, hostSkills);
  // The flat order the keyboard walks, from the same function the menu draws
  // from: a highlight is only meaningful if it indexes the rows on screen.
  const slashItems = remoteSlashItems(slashGroups);
  const [activeItem, setActiveItem] = useState(0);

  // A draft that arrived from somewhere other than a keystroke — the skill card's
  // insert, a cleared field — has to be measured too, or the box keeps the height
  // of the text it no longer holds.
  useEffect(() => {
    if (textareaRef.current)
      growToContent(textareaRef.current);
  }, [text]);

  /**
   * Take the chosen item.
   *
   * A command runs and takes its word out of the draft; a skill leaves its own
   * name behind for the host to resolve when the prompt arrives, which is the
   * only thing `/name` means to the host.
   */
  function chooseSlashItem(item: RemoteSlashItem) {
    setActiveItem(0);
    if (item.kind === "command") {
      setText(withoutSlashWord(text));
      item.command.run();
      return;
    }
    setText(insertRemoteSkill(text, item.skill.name));
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
    const highlighted = slashItems[activeItem];
    if (highlighted) {
      chooseSlashItem(highlighted);
      return;
    }
    void send();
  }

  return (
    // The same shell as this app's own composer (`Composer.tsx`): the caller
    // supplies the layout (`className`), and what is fixed here is the shell
    // itself — `rounded-lg`/`border-line`/`p-2`/`shadow-panel`/`backdrop-blur`,
    // which is the box a reader recognises as "where I type".
    <form
      className={cn(
        "relative rounded-lg border border-line bg-surface/95 p-2 shadow-panel backdrop-blur",
        className,
      )}
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      {query !== null
        ? (
            <RemoteSlashMenu
              activeIndex={activeItem}
              groups={slashGroups}
              onChoose={chooseSlashItem}
            />
          )
        : null}
      {error ? <p className="mb-2 text-xs text-danger">{error}</p> : null}
      {pendingPrompt
        ? (
            <p className="mb-2 text-xs text-warning">
              {t("promptPending")}
            </p>
          )
        : null}
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
          user cannot see is one they cannot remove. The local composer's own
          chip — same height, same radius, same transparent border (the chip is
          not a control), the same 16px remove button, and the same
          middle-truncation so the suffix stays visible while the stem shrinks. */}
      {uploads.length > 0
        ? (
            <div className="mb-2 flex flex-wrap items-center gap-1.5 px-1">
              {uploads.map((upload) => {
                // Middle-truncate: the stem shrinks with an ellipsis while the
                // extension stays pinned, so "pasted-…0000.png" keeps its suffix.
                const { stem, ext } = splitFileName(upload.name);
                return (
                  <span
                    className="inline-flex max-w-64 items-center gap-1.5 rounded-md border border-transparent bg-surface-subtle px-2 py-1 text-xs text-ink-soft"
                    key={upload.uploadId}
                    title={upload.name}
                  >
                    <Paperclip aria-hidden className="size-3 shrink-0" />
                    <span className="flex min-w-0 items-center">
                      <span className="truncate">{stem}</span>
                      {ext ? <span className="shrink-0">{ext}</span> : null}
                    </span>
                    <button
                      aria-label={t("removeAttachment", { name: upload.name })}
                      className="inline-flex size-4 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface hover:text-ink"
                      onClick={() => setUploads(current => current.filter(item => item.uploadId !== upload.uploadId))}
                      type="button"
                    >
                      <X className="size-3" />
                    </button>
                  </span>
                );
              })}
            </div>
          )
        : null}
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
        // `block`, not the default `inline-block`: an inline-block textarea sits
        // on a text baseline, so its line box keeps room for the descender below
        // it — about 6px of empty space the local editor (a block-level `div`)
        // does not pay. The two composers were then 6px apart in height with
        // nothing visible to explain it.
        className="block max-h-[40vh] min-h-14 w-full resize-none overflow-y-auto whitespace-pre-wrap wrap-break-word bg-transparent px-2 py-1 text-sm leading-5 text-ink outline-none placeholder:text-ink-muted"
        onChange={(event) => {
          setText(event.target.value);
          growToContent(event.currentTarget);
        }}
        onPaste={event => void pasteFiles(event)}
        onKeyDown={(event) => {
          // The slash menu owns the arrows and Enter while it is open: the
          // word being typed is a command, not a sentence.
          if (query !== null && slashItems.length > 0) {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setActiveItem(index => (index + 1) % slashItems.length);
              return;
            }
            if (event.key === "ArrowUp") {
              event.preventDefault();
              setActiveItem(index => (index - 1 + slashItems.length) % slashItems.length);
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
        placeholder={t(canCompact ? "composerPlaceholder" : "composerPlaceholderSkills")}
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
    </form>
  );
}
