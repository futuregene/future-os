import type { FormEvent } from "react";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { MessageAttachment } from "./agentThreadTypes";
import type { MentionEditorHandle } from "./MentionEditor";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowUp, ChevronDown, Paperclip, ShieldCheck, ShieldOff, ShieldQuestion, Square, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { SelectMenu, SelectMenuItem } from "../../components/ui/SelectMenu";
import { modelKey, modelLabel, modelOption, normalizeThinkingLevel, thinkingLevels } from "../../integrations/agent/agentClient";
import { useProviderNames } from "../../integrations/agent/useProviderNames";
import { savePastedImage } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { formatBytes } from "../../lib/format";
import { onFutureEvent } from "../../lib/futureEvents";
import { isMacOS } from "../../lib/platform";
import { classifyAttachment, fileNameFromPath, imageExtensionFromMime, isDraggableAttachment, MAX_IMAGES_PER_TURN, READ_SOURCE_MAX_BYTES } from "./attachments";
import { clearComposerDraft, loadComposerDraft, saveComposerDraft } from "./composerDraft";
import { MentionEditor } from "./MentionEditor";

/** Approval-tier order for the composer dropdown (sandbox is macOS-only). */
const APPROVAL_TIERS: ApprovalTier[] = ["manual", "sandbox", "off"];

/**
 * Icon per approval tier, shared between the dropdown rows and the trigger so
 * the button always mirrors the selected tier. Shield family: question (asks
 * you) → check (sandboxed) → off (unrestricted).
 */
function tierIcon(tier: ApprovalTier, className: string) {
  if (tier === "sandbox")
    return <ShieldCheck className={className} />;
  if (tier === "off")
    return <ShieldOff className={className} />;
  return <ShieldQuestion className={className} />;
}

export interface ComposerSendPayload {
  attachments: MessageAttachment[];
  content: string;
}

interface ComposerProps {
  /**
   * Sync send (thread path): the message renders as an optimistic bubble, so
   * the composer clears immediately. Async send (new-conversation path): the
   * message has nowhere to live until the thread exists, so the composer only
   * clears after the promise resolves — a failed creation keeps the draft for
   * retry (the caller surfaces the error itself, e.g. via toast).
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
  onChangeApprovalTier?: (value: ApprovalTier) => void;
  /** A reply is streaming: the send button becomes an interrupt button. */
  sending?: boolean;
  /** Interrupt the in-flight reply (only meaningful while `sending`). */
  onAbort?: () => void;
  placeholder?: string;
  textareaClassName?: string;
  workspaceId?: string | null;
  /**
   * Identifies the conversation whose unsent input (text, mentions, attachments)
   * this composer holds. The draft is scoped to this key in sessionStorage, so
   * switching conversations never carries content across; undefined disables
   * draft persistence (e.g. no active thread).
   */
  draftKey?: string;
}

export function Composer({
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
  onChangeApprovalTier,
  sending,
  onAbort,
  placeholder,
  textareaClassName,
  workspaceId,
  draftKey,
}: ComposerProps) {
  const { t } = useTranslation("agent");
  const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  // Drag-over verdict: null (no drag), "accept" (droppable), "reject"
  // (unsupported type — pre-validated on `enter` so the drop zone shows the
  // rejection before release, and the drop is silently ignored).
  const [dragState, setDragState] = useState<"accept" | "reject" | null>(null);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [thinkingMenuOpen, setThinkingMenuOpen] = useState(false);
  const [approvalMenuOpen, setApprovalMenuOpen] = useState(false);
  const providerNames = useProviderNames();
  // The editor is non-controlled (see MentionEditor); we only mirror its empty
  // state to enable/disable the send button.
  const [inputEmpty, setInputEmpty] = useState(true);
  // An async onSend is in flight (see ComposerProps.onSend) — block re-submits
  // until it settles.
  const [sendPending, setSendPending] = useState(false);
  const editorRef = useRef<MentionEditorHandle | null>(null);

  // ── Per-conversation draft (sessionStorage, keyed by draftKey) ──────────────
  // Live mirrors so the persist path always reads current values regardless of
  // render timing (the editor text is read from the live DOM on save).
  const attachmentsRef = useRef(attachments);
  attachmentsRef.current = attachments;
  const draftKeyRef = useRef(draftKey);
  draftKeyRef.current = draftKey;
  // Last known editor text (getContent markdown) — a fallback for when the
  // editor ref is gone (e.g. reading during unmount).
  const lastTextRef = useRef("");
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
  }, []);

  // Load this conversation's draft when it becomes active. Continuous saves
  // (editor onChange + the attachments effect) keep the outgoing conversation's
  // draft current, so switching in never needs to flush the previous one here.
  useEffect(() => {
    restoringRef.current = true;
    const draft = draftKey ? loadComposerDraft(draftKey) : null;
    const text = draft?.text ?? "";
    editorRef.current?.restore(text);
    lastTextRef.current = text;
    setAttachments(draft?.attachments ?? []);
    setAttachError(null);
  }, [draftKey]);

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
  // Only offer/accept images when the active model advertises image input.
  // Unknown model (not in the catalog yet) → allow, to avoid over-restricting.
  const allowImages = activeModel ? activeModel.supportsImages !== false : true;
  const activeThinkingLevel = normalizeThinkingLevel(thinkingLevel);
  // Localized thinking-level label; unknown levels fall back to the raw value.
  const thinkingLevelLabel = (level: string) => t(`composer.thinkingLevelLabels.${level}`, { defaultValue: level });

  // The file tree's "attach to context" action inserts a mention pill into the
  // active thread's composer. editorRef is stable, so subscribe once.
  useEffect(() => onFutureEvent("attach-file-to-context", (detail) => {
    editorRef.current?.insertMention(detail);
  }), []);

  // Autofocus so the user can type immediately: on mount, when switching
  // conversations (draftKey changes), and when the composer re-enables after a
  // send settles (disabled: true → false). A disabled editor is
  // contentEditable=false and can't hold a caret, so only focus while enabled.
  useEffect(() => {
    if (!disabled)
      editorRef.current?.focus();
  }, [disabled, draftKey]);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    submitValue();
  }

  function submitValue() {
    const trimmed = (editorRef.current?.getContent() ?? "").trim();
    if ((!trimmed && attachments.length === 0) || disabled || sendPending)
      return;
    const clearComposer = () => {
      editorRef.current?.clear();
      setAttachments([]);
      setAttachError(null);
      lastTextRef.current = "";
      if (draftKeyRef.current)
        clearComposerDraft(draftKeyRef.current);
    };
    const result = onSend({ attachments, content: trimmed });
    if (result) {
      // Async send: clear only on success so a failure keeps the draft
      // (rationale on ComposerProps.onSend). The caller reports the error.
      setSendPending(true);
      result
        .then(clearComposer)
        .catch(() => {})
        .finally(() => setSendPending(false));
      return;
    }
    clearComposer();
  }

  const addAttachmentPaths = useCallback(async (paths: string[]) => {
    const classified = await Promise.all(
      paths.map(async path => ({ path, result: await classifyAttachment(path) })),
    );
    // Compute next/rejected against the current attachments, then call both
    // setters — never call setAttachError inside a setAttachments updater
    // (updaters must be pure; StrictMode/concurrent React may run them twice).
    // Classification is asynchronous and several sources (picker, paste, drag)
    // may finish out of order. Merge into the live ref, not the render-time
    // closure, so a later completion cannot overwrite an earlier one.
    const next = [...attachmentsRef.current];
    const rejected: string[] = [];
    for (const { path, result } of classified) {
      if (next.some(attachment => attachment.path === path))
        continue;
      if (result.kind === null) {
        rejected.push(t("composer.attachRejectedReason", { name: fileNameFromPath(path), reason: result.reason }));
        continue;
      }
      // Only images are limited (count + model support). Every other file type
      // is unlimited — the agent reads local paths on demand with its own tools.
      if (result.kind === "image") {
        if (!allowImages) {
          rejected.push(t("composer.attachRejectedNoImage", { name: fileNameFromPath(path) }));
          continue;
        }
        const imageCount = next.filter(attachment => attachment.kind === "image").length;
        if (imageCount >= MAX_IMAGES_PER_TURN) {
          rejected.push(t("composer.attachRejectedLimit", { name: fileNameFromPath(path), count: MAX_IMAGES_PER_TURN }));
          continue;
        }
      }
      next.push({ kind: result.kind, name: fileNameFromPath(path), path });
    }
    attachmentsRef.current = next;
    setAttachments(next);
    setAttachError(rejected.length > 0 ? t("composer.attachIgnored", { items: rejected.join("，") }) : null);
  }, [allowImages, t]);

  async function attachImageFiles(files: File[]) {
    // Save every file first, then attach in ONE addAttachmentPaths call:
    // calling it per file inside the loop reuses the same closure over the
    // pre-paste `attachments`, so each iteration's setAttachments overwrites
    // the previous one and only the last image survives.
    const saved: string[] = [];
    for (const file of files) {
      if (file.size > READ_SOURCE_MAX_BYTES) {
        setAttachError(t("composer.attachIgnored", {
          items: t("composer.attachRejectedReason", {
            name: file.name,
            reason: t("attachment.imageTooLarge", { max: formatBytes(READ_SOURCE_MAX_BYTES) }),
          }),
        }));
        continue;
      }
      try {
        const buffer = await file.arrayBuffer();
        const result = await savePastedImage({
          bytes: Array.from(new Uint8Array(buffer)),
          extension: imageExtensionFromMime(file.type) ?? "png",
        });
        saved.push(result.path);
      }
      catch {
        // Ignore a single failed paste; other clipboard items still attach.
      }
    }
    if (saved.length > 0)
      await addAttachmentPaths(saved);
  }

  async function handleAttachFiles() {
    if (disabled)
      return;

    // Any file type is acceptable (the agent reads paths with its own tools), so
    // the picker offers no extension filter. Images picked for a text-only model
    // are rejected post-selection in addAttachmentPaths.
    const selected = await open({
      multiple: true,
      title: t("composer.attachDialogTitle"),
    });
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    if (paths.length === 0)
      return;

    await addAttachmentPaths(paths);
  }

  function removeAttachment(path: string) {
    const next = attachmentsRef.current.filter(attachment => attachment.path !== path);
    attachmentsRef.current = next;
    setAttachments(next);
  }

  // Held in a ref so the webview drag listener below doesn't re-subscribe on
  // every attachment change (addAttachmentPaths closes over `attachments`).
  const addAttachmentPathsRef = useRef(addAttachmentPaths);
  addAttachmentPathsRef.current = addAttachmentPaths;
  // Same reason: the drag listener reads the live image-allowance without
  // re-subscribing when the active model (hence `allowImages`) changes.
  const allowImagesRef = useRef(allowImages);
  allowImagesRef.current = allowImages;

  useEffect(() => {
    if (disabled)
      return;

    let active = true;
    let dispose: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "enter") {
          // `enter` carries the paths — pre-validate by extension so the drop
          // zone shows accept vs. reject before the user releases.
          const droppable = event.payload.paths.some(path => isDraggableAttachment(path, allowImagesRef.current));
          setDragState(droppable ? "accept" : "reject");
        }
        else if (event.payload.type === "over") {
          // `over` has no paths; keep the verdict decided on `enter`.
          setDragState(prev => prev ?? "accept");
        }
        else if (event.payload.type === "leave") {
          setDragState(null);
        }
        else if (event.payload.type === "drop") {
          setDragState(null);
          // Forward only extension-acceptable files; unsupported ones (e.g.
          // .xlsx) are silently ignored — no "已忽略" toast for a file the drop
          // zone already flagged as rejected.
          const accepted = event.payload.paths.filter(path => isDraggableAttachment(path, allowImagesRef.current));
          if (accepted.length > 0)
            void addAttachmentPathsRef.current(accepted);
        }
      })
      .then((unlisten) => {
        if (active)
          dispose = unlisten;
        else
          unlisten();
      });

    return () => {
      active = false;
      dispose?.();
      setDragState(null);
    };
  }, [disabled]);

  return (
    <form
      className={cn(
        "relative rounded-lg border border-line bg-surface/95 p-2 shadow-panel backdrop-blur",
        dragState === "accept" && "ring-2 ring-focus",
        dragState === "reject" && "ring-2 ring-danger-line",
        className,
      )}
      onSubmit={handleSubmit}
    >
      {dragState === "reject"
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
              {attachments.map(attachment => (
                <span
                  className="inline-flex max-w-64 items-center gap-1.5 rounded-md bg-surface-subtle px-2 py-1 text-xs text-ink-soft"
                  key={attachment.path}
                  title={attachment.path}
                >
                  <Paperclip className="size-3 shrink-0" />
                  <span className="truncate">{attachment.name}</span>
                  <button
                    aria-label={t("composer.removeAttachment", { name: attachment.name })}
                    className="inline-flex size-4 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface hover:text-ink"
                    onClick={() => removeAttachment(attachment.path)}
                    type="button"
                  >
                    <X className="size-3" />
                  </button>
                </span>
              ))}
            </div>
          )
        : null}
      <MentionEditor
        ref={editorRef}
        className={textareaClassName}
        workspaceId={workspaceId}
        disabled={disabled}
        placeholder={placeholder ?? t("composer.placeholder")}
        onSubmit={submitValue}
        onEmptyChange={setInputEmpty}
        onChange={saveDraft}
        onPasteImages={files => void attachImageFiles(files)}
      />
      {attachError
        ? <div className="px-1 pb-1 text-xs text-warning">{attachError}</div>
        : null}
      <div className="flex items-center justify-between pt-1">
        <div className="flex items-center gap-1">
          <button
            className="inline-flex size-7 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
            disabled={disabled}
            onClick={() => void handleAttachFiles()}
            type="button"
            aria-label={t("composer.attachFiles")}
            title={allowImages ? t("composer.attachFilesHint") : t("composer.attachFilesHintNoImage")}
          >
            <Paperclip className="size-3.5" />
          </button>
          {onChangeApprovalTier
            ? (
                <SelectMenu
                  open={approvalMenuOpen}
                  onDismiss={() => setApprovalMenuOpen(false)}
                  panelClassName="w-64 overflow-hidden"
                  trigger={(
                    <button
                      className="inline-flex h-7 max-w-40 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                      onClick={() => {
                        setModelMenuOpen(false);
                        setThinkingMenuOpen(false);
                        setApprovalMenuOpen(open => !open);
                      }}
                      type="button"
                      title={t("composer.approval")}
                    >
                      {tierIcon(approvalTier ?? "off", "size-3 shrink-0")}
                      <span className="truncate">{t(`composer.approvalTier.${approvalTier ?? "off"}`)}</span>
                      <ChevronDown className="size-3 shrink-0" />
                    </button>
                  )}
                >
                  {APPROVAL_TIERS.filter(tier => tier !== "sandbox" || isMacOS).map(tier => (
                    <SelectMenuItem
                      className="py-1.5"
                      key={tier}
                      selected={(approvalTier ?? "off") === tier}
                      onSelect={() => {
                        onChangeApprovalTier(tier);
                        setApprovalMenuOpen(false);
                      }}
                    >
                      {tierIcon(tier, "size-4 shrink-0 text-ink-soft")}
                      <span className="min-w-0 flex-1 space-y-0.5">
                        <span className="block truncate font-medium leading-tight text-ink">{t(`composer.approvalTier.${tier}`)}</span>
                        <span className="block text-xs leading-tight text-ink-muted">
                          {tier === "off"
                            ? <Trans t={t} i18nKey="composer.approvalTierDesc.off" components={{ em: <span className="font-semibold" /> }} />
                            : t(`composer.approvalTierDesc.${tier}`)}
                        </span>
                      </span>
                    </SelectMenuItem>
                  ))}
                </SelectMenu>
              )
            : null}
        </div>
        <div className="flex items-center gap-2">
          <SelectMenu
            className="hidden md:block"
            open={modelMenuOpen}
            onDismiss={() => setModelMenuOpen(false)}
            panelClassName="max-h-[40vh] w-56 overflow-y-auto"
            trigger={(
              <button
                className="inline-flex h-7 max-w-48 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                onClick={() => {
                  setThinkingMenuOpen(false);
                  setApprovalMenuOpen(false);
                  setModelMenuOpen(open => !open);
                }}
                type="button"
                title={t("composer.model")}
              >
                <span className="truncate">{modelLabel(activeModelId, modelOptions) ?? t("common:modelFallback")}</span>
                <ChevronDown className="size-3 shrink-0" />
              </button>
            )}
          >
            {modelOptions.length === 0
              ? (
                  <div className="px-3 py-2 text-sm text-ink-muted">
                    {modelsEmptyReason === "all_disabled"
                      ? t("composer.allModelsDisabled")
                      : t("composer.startAgentForModels")}
                  </div>
                )
              : null}
            {modelOptions.map(model => (
              <SelectMenuItem
                className="py-1"
                key={`${model.provider}/${model.id}`}
                selected={model === activeModel}
                onSelect={() => {
                  onModelChange?.(modelKey(model));
                  setModelMenuOpen(false);
                }}
              >
                <span className="min-w-0 flex-1 space-y-0.5">
                  <span className="block truncate font-medium leading-tight text-ink">{model.label}</span>
                  <span className="block truncate text-xs leading-tight text-ink-muted">
                    {providerNames[model.provider] ?? model.provider}
                  </span>
                </span>
              </SelectMenuItem>
            ))}
          </SelectMenu>
          <SelectMenu
            className="hidden md:block"
            open={thinkingMenuOpen}
            onDismiss={() => setThinkingMenuOpen(false)}
            panelClassName="w-40 overflow-hidden"
            trigger={(
              <button
                className="inline-flex h-7 max-w-40 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                onClick={() => {
                  setModelMenuOpen(false);
                  setApprovalMenuOpen(false);
                  setThinkingMenuOpen(open => !open);
                }}
                type="button"
                title={t("composer.thinkingLevel")}
              >
                <span className="truncate">{thinkingLevelLabel(activeThinkingLevel)}</span>
                <ChevronDown className="size-3 shrink-0" />
              </button>
            )}
          >
            {thinkingLevels.map(level => (
              <SelectMenuItem
                key={level}
                selected={activeThinkingLevel === level}
                onSelect={() => {
                  onThinkingLevelChange?.(level);
                  setThinkingMenuOpen(false);
                }}
              >
                <span className="min-w-0 flex-1 truncate font-medium text-ink">{thinkingLevelLabel(level)}</span>
              </SelectMenuItem>
            ))}
          </SelectMenu>
          {sending
            ? (
                <button
                  className="inline-flex size-7 items-center justify-center rounded-md bg-ink text-surface transition-colors hover:bg-ink-soft"
                  onClick={() => onAbort?.()}
                  type="button"
                  aria-label={t("composer.stop")}
                  title={t("composer.stop")}
                >
                  <Square className="size-3 fill-current" />
                </button>
              )
            : (
                <button
                  className="inline-flex size-7 items-center justify-center rounded-md bg-accent text-white transition-colors hover:bg-accent-hover disabled:bg-accent-disabled"
                  disabled={(inputEmpty && attachments.length === 0) || disabled || sendPending}
                  type="submit"
                  aria-label={t("composer.send")}
                  title={t("composer.send")}
                >
                  <ArrowUp className="size-3.5" />
                </button>
              )}
        </div>
      </div>
    </form>
  );
}
