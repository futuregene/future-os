import type { MessageAttachment } from "@future-os/thread-projection";
import type { FormEvent } from "react";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { ContextToolOption, MentionEditorHandle, SkillMentionOption } from "./MentionEditor";
import type { SessionMentionOption } from "./sessionMention";
import { Paperclip, TriangleAlert, X } from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { modelKey, modelOption } from "../../integrations/agent/agentClient";
import { useSkillCatalog } from "../../integrations/skills/useSkillCatalog";
import { cn } from "../../lib/cn";
import { emitFutureEvent, onFutureEvent } from "../../lib/futureEvents";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { useOperationLifetime } from "../../lib/useOperationLifetime";
import { splitFileName } from "./attachments";
import { ComposerControls } from "./composer/ComposerControls";
import { useComposerAttachments } from "./composer/useComposerAttachments";
import { clearComposerDraft, loadComposerDraft, saveComposerDraft } from "./composerDraft";
import { MentionEditor } from "./MentionEditor";
import { SkillRecommendCard } from "./SkillRecommendCard";

export interface ComposerSendPayload {
  attachments: MessageAttachment[];
  content: string;
}

/** Drag-over verdict for the drop zone. */
export type ComposerDragState = "accept" | "reject" | null;

/** A skill recommendation shown above the composer (new-conversation first turn). */
export interface SkillRecommendationCard {
  name: string;
  description: string;
}

/**
 * Optional skill-recommendation wiring (new-conversation first turn only).
 * When present, submit is first routed through `onEvaluate`; a returned card
 * holds the draft until the user installs the skill, presses the card's
 * "send without it" button, or sends again (a plain send counts as the
 * latter).
 */
export interface SkillRecommendationProp {
  /** The card to show, or null. */
  card: SkillRecommendationCard | null;
  /**
   * Evaluate the draft for a recommendation. Resolve with the card to show
   * (submission is held), or null to submit normally. Never rejects.
   */
  onEvaluate: (draft: string) => Promise<SkillRecommendationCard | null>;
  /**
   * Install the recommended skill. Resolve true on success; the composer then
   * appends `/name` to the draft and sends. Resolve false to leave the card up.
   */
  onInstall: (skill: SkillRecommendationCard) => Promise<boolean>;
  /** Dismiss the card and send the original draft. */
  onDismiss: () => void;
}

interface ComposerProps {
  /**
   * Send the message. The composer hands the draft over and empties itself as
   * soon as this returns (the caller shows the message optimistically, so the
   * box must not keep a second copy of it). Resolve when the message is
   * accepted, not when the assistant finishes; rejecting restores the
   * submitted draft — unless the user typed something new in the meantime, in
   * which case their draft wins and the rejected message stays readable in the
   * conversation. New-conversation callers resolve once the prompt is staged
   * in its newly created thread.
   */
  onSend: (payload: ComposerSendPayload) => void | Promise<void>;
  className?: string;
  disabled?: boolean;
  modelId?: string;
  modelOptions: AgentModelOption[];
  /** Why the picker list is empty, to tailor its empty-state copy. */
  modelsEmptyReason?: "no_models" | "all_disabled";
  onModelChange?: (modelId: string) => void;
  thinkingLevel?: string;
  onThinkingLevelChange?: (thinkingLevel: string) => void;
  approvalTier?: ApprovalTier;
  futureSessionStatus?: string;
  onChangeApprovalTier?: (value: ApprovalTier) => void;
  /**
   * A reply is streaming. The send button becomes an interrupt button and
   * submission (button + Enter) is blocked until the stream ends — but the
   * editor stays editable so the user can draft the next message early. Keep this
   * separate from `disabled`: `disabled` locks the editor, so folding the
   * streaming state into it (as the call site used to do) would prevent that
   * early drafting.
   */
  sending?: boolean;
  /** Interrupt the in-flight reply (only meaningful while `sending`). */
  onAbort?: () => void;
  /** Run a standalone compaction for the current conversation. */
  onCompactContext?: () => void | Promise<void>;
  /** A compaction reported by the agent is still running (survives reloads). */
  compactionInProgress?: boolean;
  /** First-turn skill recommendation (new-conversation only). */
  skillRecommendation?: SkillRecommendationProp;
  placeholder?: string;
  textareaClassName?: string;
  workspaceId?: string | null;
  /**
   * Conversations offered by the `#` menu, already ordered and with the current
   * conversation excluded (see `sessionMentionOptions`). Omit/empty to disable
   * the menu.
   */
  sessionMentions?: SessionMentionOption[];
  /**
   * Identifies the conversation whose unsent input (text, mentions, attachments)
   * this composer holds. The draft is scoped to this key in sessionStorage, so
   * switching conversations never carries content across; undefined disables
   * draft persistence (e.g. no active thread).
   */
  draftKey?: string;
  /**
   * Opt out of the built-in drag highlight and report the verdict to the parent
   * instead. Used where the composer is only the top of a larger card (e.g. the
   * new-conversation form has a footer bar below it): the parent draws the ring
   * around the whole card so the highlight isn't cut off at the composer's edge.
   */
  onDragStateChange?: (state: ComposerDragState) => void;
}

function ComposerImpl({
  onSend,
  className,
  disabled,
  modelId,
  modelOptions,
  modelsEmptyReason,
  onModelChange,
  thinkingLevel,
  onThinkingLevelChange,
  approvalTier,
  futureSessionStatus = "checking",
  onChangeApprovalTier,
  sending,
  onAbort,
  onCompactContext,
  compactionInProgress,
  skillRecommendation,
  placeholder,
  textareaClassName,
  workspaceId,
  sessionMentions,
  draftKey,
  onDragStateChange,
}: ComposerProps) {
  const { t, i18n } = useTranslation("agent");
  // The editor is non-controlled (see MentionEditor); we only mirror its empty
  // state to enable/disable the send button.
  const [inputEmpty, setInputEmpty] = useState(true);
  // An async onSend is in flight (see ComposerProps.onSend) — block re-submits
  // until it settles.
  const [sendPending, setSendPending] = useState(false);
  const [contextActionPending, setContextActionPending] = useState(false);
  const compactionPending = contextActionPending || compactionInProgress;
  // True while a recommend round-trip holds submission. Both a ref and a state:
  // the ref guards re-entrant submits synchronously (`submitValue` is called
  // from an event, not a render), the state drives the locked input + spinner
  // so the wait is visible instead of looking like a dead composer.
  const recommendPendingRef = useRef(false);
  const [recommendPending, setRecommendPending] = useState(false);
  // Tracks that the user already acted on the current card (install/dismiss),
  // so the follow-up send isn't blocked by the still-mounted card (the parent's
  // setState that clears it hasn't re-rendered yet).
  const cardHandledRef = useRef(false);
  // The draft text already sent to the recommender, so one draft is asked about
  // at most once (a repeat would spend a second call for the same message).
  const evaluatedDraftRef = useRef<string | null>(null);
  // True while the recommended skill is installing (disables the card buttons).
  const [installingSkill, setInstallingSkill] = useState(false);
  const editorRef = useRef<MentionEditorHandle | null>(null);
  const captureOperation = useOperationLifetime(draftKey);
  const { attachments, attachmentsRef, setAttachments, attachError, setAttachError, dragState, addAttachmentPaths, attachPastedFiles, handleAttachFiles, removeAttachment }
    = useComposerAttachments({ disabled, onDragStateChange, captureOperation });
  const sendStateRef = useCommittedRef({ disabled, sending, compactionPending, onSend });
  useEffect(() => {
    recommendPendingRef.current = false;
    evaluatedDraftRef.current = null;
    cardHandledRef.current = false;
    setRecommendPending(false);
    setInstallingSkill(false);
    setSendPending(false);
    setContextActionPending(false);
  }, [draftKey]);

  const contextTools = useMemo<ContextToolOption[]>(() => {
    if (!onCompactContext || sending || compactionPending)
      return [];
    return [{
      id: "compact",
      name: t("composer.compactContext"),
      description: t("composer.compactContextDescription"),
      searchText: "compact compaction compress context 压缩 上下文",
    }];
  }, [compactionPending, onCompactContext, sending, t]);
  const handleContextToolSelect = useCallback((toolId: string) => {
    if (toolId !== "compact" || !onCompactContext || compactionPending)
      return;
    const isCurrent = captureOperation();
    const result = onCompactContext();
    if (result) {
      setContextActionPending(true);
      result.catch(() => {}).finally(() => {
        if (isCurrent())
          setContextActionPending(false);
      });
    }
  }, [captureOperation, compactionPending, onCompactContext]);

  const catalog = useSkillCatalog();
  const skills = useMemo<SkillMentionOption[]>(() => {
    const useZh = i18n.language !== "en";
    const localized = new Map(catalog.catalogue.map(skill => [skill.id, skill]));
    return catalog.installed.map((skill) => {
      const nameZh = skill.nameZh || localized.get(skill.id)?.nameZh || null;
      const descriptionZh = skill.descriptionZh || localized.get(skill.id)?.descriptionZh || null;
      return {
        name: skill.name,
        description: useZh ? descriptionZh || skill.description : skill.description,
        nameZh,
        descriptionZh,
      };
    });
  }, [catalog, i18n.language]);

  // ── Per-conversation draft (sessionStorage, keyed by draftKey) ──────────────
  // Live mirrors so the persist path always reads current values regardless of
  // render timing (the editor text is read from the live DOM on save).
  const draftKeyRef = useCommittedRef(draftKey);
  // Last known editor text (getContent markdown) — a fallback for when the
  // editor ref is gone (e.g. reading during unmount).
  const lastTextRef = useRef("");
  // The markdown most recently restored from storage. Used to re-run the
  // skill-pill upgrade once the installed-skills list loads (a draft's `/name`
  // tokens can't become pills until the skills are known).
  const restoredTextRef = useRef("");
  // Set while applying a restore, so the attachments effect below doesn't
  // re-persist the just-loaded draft with values that haven't settled yet.
  const restoringRef = useRef(false);

  const saveDraft = useCallback(() => {
    const key = draftKeyRef.current;
    if (!key)
      return;
    const text = editorRef.current ? editorRef.current.getContent() : lastTextRef.current;
    lastTextRef.current = text;
    saveComposerDraft(key, { attachments: attachmentsRef.current, text });
  }, [attachmentsRef, draftKeyRef]);

  // Load this conversation's draft when it becomes active. Continuous saves
  // (editor onChange + the attachments effect) keep the outgoing conversation's
  // draft current, so switching in never needs to flush the previous one here.
  useEffect(() => {
    restoringRef.current = true;
    const draft = draftKey ? loadComposerDraft(draftKey) : null;
    const text = draft?.text ?? "";
    editorRef.current?.restore(text);
    lastTextRef.current = text;
    restoredTextRef.current = text;
    setAttachments(draft?.attachments ?? []);
    setAttachError(null);
  }, [draftKey, setAttachError, setAttachments]);

  // The `/`-menu skills load after mount, so a freshly restored draft with a
  // `/name` token (e.g. the Skills page 「试试」 prefill) first renders as plain
  // text. Re-run the restore once skills arrive — but only while the editor
  // still holds the untouched restored text, so user edits are never clobbered.
  useEffect(() => {
    if (skills.length === 0)
      return;
    const current = editorRef.current?.getContent() ?? "";
    if (!restoredTextRef.current || current !== restoredTextRef.current)
      return;
    editorRef.current?.restore(restoredTextRef.current);
  }, [skills]);

  // Persist attachment edits (skip the restore-driven update, which the effect
  // above already loaded from storage).
  useEffect(() => {
    if (restoringRef.current) {
      restoringRef.current = false;
      return;
    }
    saveDraft();
  }, [attachments, saveDraft]);

  const activeModelId = modelId || (modelOptions[0] ? modelKey(modelOptions[0]) : "");
  const activeModel = modelOption(activeModelId, modelOptions);
  // Whether the active model consumes images as vision (inline image blocks).
  // Images are always *accepted* now — a model that can't take them still gets
  // the file path and can read it with its tools — but when this is false the
  // attachment chip is flagged so the user knows the image may not be understood.
  // Unknown model (not in the catalog yet) → treat as vision-capable.
  const supportsImages = activeModel ? activeModel.supportsImages !== false : true;
  // The file tree's "attach to context" action inserts a mention pill into the
  // active thread's composer. editorRef is stable, so subscribe once.
  useEffect(() => onFutureEvent("attach-file-to-context", (detail) => {
    editorRef.current?.insertMention(detail);
  }), []);

  // The terminal panel asks for the caret when it collapses: the panel is gone
  // from the layout, so leaving focus on a hidden terminal would swallow the
  // next keystrokes. Declined while this composer cannot hold a caret
  // (`disabled` is a contentEditable=false editor).
  useEffect(() => onFutureEvent("focus-composer", () => {
    if (!disabled)
      editorRef.current?.focus();
  }), [disabled]);

  // Autofocus so the user can type immediately: on mount, when switching
  // conversations (draftKey changes), and when a send settles. The streaming
  // lock lives on `sending` now (the editor stays editable mid-stream so the
  // user can draft the next message), so we key off `sending` here: don't steal
  // focus while a reply streams, but re-focus the moment it ends. A hard-
  // disabled editor is contentEditable=false and can't hold a caret, so only
  // focus while enabled.
  useEffect(() => {
    if (!disabled && !sending)
      editorRef.current?.focus();
  }, [disabled, sending, draftKey]);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    submitValue();
  }

  function submitValue() {
    const trimmed = (editorRef.current?.getContent() ?? "").trim();
    // Block submission while a reply streams (the send button is already an
    // abort button then), while context compaction is running, and while an
    // async send is in flight. The editor intentionally remains enabled so the
    // next message can still be drafted during either operation.
    if (
      (!trimmed && attachments.length === 0)
      || disabled
      || sendPending
      || sending
      || compactionPending
      || recommendPendingRef.current
    ) {
      return;
    }

    // Skill recommendation: hold the draft while we ask the recommender. A
    // returned card keeps the draft unsubmitted until the user acts on it
    // (install, dismiss, or send again); anything else (timeout, no match,
    // error) sends normally.
    const reco = skillRecommendation;
    if (reco) {
      // A card is on screen and not yet acted on: a plain send means "send
      // without it" — the same thing the card's secondary button does. The
      // send button stays live here (only the recommender wait disables it),
      // so returning silently would read as a broken button.
      if (reco.card && !cardHandledRef.current) {
        dismissRecommendedSkill();
        return;
      }
      // Ask once per draft. `evaluatedDraftRef` records that this draft has been
      // asked, so the fall-through below cannot re-enter this branch and spend a
      // second call for the same message.
      if (!cardHandledRef.current && evaluatedDraftRef.current !== trimmed) {
        const isCurrent = captureOperation();
        evaluatedDraftRef.current = trimmed;
        recommendPendingRef.current = true;
        setRecommendPending(true);
        reco
          .onEvaluate(trimmed)
          .then((card) => {
            if (isCurrent() && !card)
              sendNow(trimmed);
          })
          .catch(() => {
            if (isCurrent())
              sendNow(trimmed);
          })
          .finally(() => {
            if (!isCurrent())
              return;
            recommendPendingRef.current = false;
            setRecommendPending(false);
          });
        return;
      }
    }

    sendNow();
  }

  /**
   * The real send path, reached only once no recommendation is holding the
   * draft. Kept separate from `submitValue` so the intercept can fall through to
   * it without re-running the evaluation — calling `submitValue` recursively
   * would both re-enter the intercept and be refused by the in-flight guard
   * (`recommendPendingRef`, still set while the promise chain is resolving),
   * silently swallowing the send.
   */
  function sendNow(evaluatedText?: string) {
    const state = sendStateRef.current;
    if (!editorRef.current || state.disabled || state.sending || state.compactionPending)
      return;
    const submittedText = editorRef.current?.getContent() ?? "";
    const trimmed = submittedText.trim();
    if (evaluatedText !== undefined && evaluatedText !== trimmed)
      return;
    const submittedDraftKey = draftKeyRef.current;
    const submittedAttachments = attachmentsRef.current;
    if (!trimmed && submittedAttachments.length === 0)
      return;
    const isCurrent = captureOperation();
    /**
     * Hand the submitted draft over: the caller appends the message to the
     * conversation the moment it is called (the optimistic bubble precedes the
     * agent handshake), so clearing on the delivery ACK instead left the same
     * message visible in the thread *and* sitting in the box for as long as
     * that handshake took — session setup, the pre-run git snapshot, or a run
     * already in flight could stretch it to seconds. The editor stays editable
     * during delivery, so a revised draft or another conversation's composer is
     * never touched: only what was actually submitted is dropped.
     */
    const dropSubmittedDraft = () => {
      if (!isCurrent() || !editorRef.current || draftKeyRef.current !== submittedDraftKey)
        return;
      editorRef.current.clear();
      lastTextRef.current = "";
      const remaining = attachmentsRef.current.filter(item => !submittedAttachments.includes(item));
      attachmentsRef.current = remaining;
      setAttachments(remaining);
      setAttachError(null);
      if (remaining.length === 0 && submittedDraftKey)
        clearComposerDraft(submittedDraftKey);
    };
    /**
     * Put the submitted draft back after a rejected delivery (rationale on
     * `ComposerProps.onSend`). A draft typed in the meantime wins, and so does
     * the conversation the user is now in; in both cases the rejected message
     * stays recoverable from its bubble in the thread.
     */
    const restoreComposer = () => {
      if (!isCurrent() || !editorRef.current || draftKeyRef.current !== submittedDraftKey)
        return;
      if ((editorRef.current.getContent() ?? "").trim().length > 0)
        return;
      editorRef.current.restore(submittedText);
      lastTextRef.current = submittedText;
      const restored = [...submittedAttachments, ...attachmentsRef.current];
      attachmentsRef.current = restored;
      setAttachments(restored);
      if (submittedDraftKey)
        saveComposerDraft(submittedDraftKey, { attachments: restored, text: submittedText });
    };
    dropSubmittedDraft();
    const result = state.onSend({ attachments: submittedAttachments, content: trimmed });
    if (result) {
      // Async send: the caller reports the failure, and only then does the
      // draft come back.
      setSendPending(true);
      result
        .catch(restoreComposer)
        .finally(() => {
          if (isCurrent())
            setSendPending(false);
        });
    }
  }

  // Reset the handled flag whenever a fresh card appears, so its actions arm.
  const recoCardName = skillRecommendation?.card?.name ?? null;
  useEffect(() => {
    cardHandledRef.current = false;
  }, [recoCardName]);

  // Install the recommended skill, then append `/name` to the draft and send.
  // The card stays up (with its buttons disabled) while the install runs; a
  // failed install leaves the card so the user can retry or dismiss.
  async function installRecommendedSkill() {
    const reco = skillRecommendation;
    if (!reco?.card || installingSkill)
      return;
    const isCurrent = captureOperation();
    setInstallingSkill(true);
    try {
      const installed = await reco.onInstall(reco.card);
      if (!isCurrent())
        return;
      if (!installed) {
        // Install failed: tell the user and keep the card up so they can retry
        // or send without the skill. The draft is untouched.
        emitFutureEvent("toast", {
          message: t("composer.skillRecommend.installFailed"),
          tone: "error",
        });
        return;
      }
      const current = editorRef.current?.getContent() ?? "";
      const separator = current.length > 0 && !current.endsWith(" ") ? " " : "";
      editorRef.current?.restore(`${current}${separator}/${reco.card.name} `);
      cardHandledRef.current = true;
      reco.onDismiss();
      // The card is handled; submitValue now takes the normal send path.
      submitValue();
    }
    finally {
      if (isCurrent())
        setInstallingSkill(false);
    }
  }

  // Dismiss the card and send the original draft unchanged.
  function dismissRecommendedSkill() {
    if (!skillRecommendation?.card)
      return;
    cardHandledRef.current = true;
    skillRecommendation.onDismiss();
    submitValue();
  }

  // When the parent handles the highlight (onDragStateChange), draw neither the
  // ring nor the reject overlay here — the parent rings the whole card instead.
  const drawOwnHighlight = !onDragStateChange;

  return (
    <form
      className={cn(
        "relative rounded-lg border border-line bg-surface/95 p-2 shadow-panel backdrop-blur",
        drawOwnHighlight && dragState === "accept" && "ring-2 ring-focus",
        drawOwnHighlight && dragState === "reject" && "ring-2 ring-danger-line",
        className,
      )}
      onSubmit={handleSubmit}
    >
      {skillRecommendation?.card
        ? (
            <SkillRecommendCard
              description={skillRecommendation.card.description}
              installing={installingSkill}
              name={skillRecommendation.card.name}
              onDismiss={dismissRecommendedSkill}
              onInstall={() => void installRecommendedSkill()}
            />
          )
        : null}
      {drawOwnHighlight && dragState === "reject"
        ? (
            <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-lg bg-danger-soft">
              <span className="rounded-md border border-danger-line bg-surface px-2.5 py-1 text-xs font-medium text-danger">
                {t("composer.dropReject")}
              </span>
            </div>
          )
        : null}
      {attachments.length > 0
        ? (
            <div className="flex flex-wrap gap-1.5 px-1 pb-2">
              {attachments.map((attachment) => {
                // Image attached to a model that can't see it: still sent (as a
                // path the agent can read), but flagged amber so the user knows
                // it may not be understood. Reverts when switching back to a
                // vision model, since `supportsImages` is recomputed on render.
                const unsupportedImage = attachment.kind === "image" && !supportsImages;
                // Middle-truncate: the stem shrinks with an ellipsis while the
                // extension stays pinned, so "pasted-…0000.png" keeps its suffix.
                const { stem, ext } = splitFileName(attachment.name);
                return (
                  <span
                    className={cn(
                      "inline-flex max-w-64 items-center gap-1.5 rounded-md border px-2 py-1 text-xs",
                      unsupportedImage
                        ? "border-warning-line bg-warning-soft text-warning"
                        : "border-transparent bg-surface-subtle text-ink-soft",
                    )}
                    key={attachment.path}
                    title={unsupportedImage ? t("composer.attachImageUnsupportedHint") : attachment.name}
                  >
                    {unsupportedImage
                      ? <TriangleAlert className="size-3 shrink-0" />
                      : <Paperclip className="size-3 shrink-0" />}
                    <span className="flex min-w-0 items-center">
                      <span className="truncate">{stem}</span>
                      {ext ? <span className="shrink-0">{ext}</span> : null}
                    </span>
                    <button
                      aria-label={t("composer.removeAttachment", { name: attachment.name })}
                      className={cn(
                        "inline-flex size-4 shrink-0 items-center justify-center rounded transition-colors",
                        unsupportedImage
                          ? "text-warning hover:bg-warning-line hover:text-warning"
                          : "text-ink-muted hover:bg-surface hover:text-ink",
                      )}
                      onClick={() => removeAttachment(attachment.path)}
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
      <MentionEditor
        ref={editorRef}
        className={textareaClassName}
        workspaceId={workspaceId}
        skills={skills}
        contextTools={contextTools}
        sessions={sessionMentions}
        // Locked while the recommender is being asked: the message about to be
        // sent must be the one that was evaluated, and a box that silently
        // ignores the send button reads as broken (the send button below spins
        // instead). `disabled` restores the caret afterwards — the autofocus
        // effect above keys off it.
        disabled={disabled || recommendPending}
        placeholder={placeholder ?? t("composer.placeholder")}
        onSubmit={submitValue}
        onEmptyChange={setInputEmpty}
        onChange={saveDraft}
        onContextToolSelect={handleContextToolSelect}
        onPasteAttachmentPaths={paths => void addAttachmentPaths(paths)}
        onPasteFiles={files => void attachPastedFiles(files)}
      />
      {attachError
        ? <div className="px-1 pb-1 text-xs text-warning">{attachError}</div>
        : null}
      <ComposerControls
        disabled={disabled}
        modelId={modelId}
        modelOptions={modelOptions}
        modelsEmptyReason={modelsEmptyReason}
        onModelChange={onModelChange}
        thinkingLevel={thinkingLevel}
        onThinkingLevelChange={onThinkingLevelChange}
        approvalTier={approvalTier}
        futureSessionStatus={futureSessionStatus}
        onChangeApprovalTier={onChangeApprovalTier}
        sending={sending}
        onAbort={onAbort}
        handleAttachFiles={handleAttachFiles}
        inputEmpty={inputEmpty}
        attachmentCount={attachments.length}
        sendPending={sendPending}
        recommendPending={recommendPending}
        compactionPending={Boolean(compactionPending)}
      />
    </form>
  );
}

/**
 * Memoized: AgentThread re-renders on every streaming push, and the composer
 * subtree (MentionEditor, the three select menus, attachment list) is
 * unaffected by those pushes — with stable props (AgentThread passes
 * useCallback-wrapped onAbort/onSend) it must not re-render at all.
 */
export const Composer = memo(ComposerImpl);
