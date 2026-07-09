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
import { isMacOS } from "../../lib/platform";
import { classifyAttachment, fileNameFromPath, imageExtensionFromMime, MAX_ATTACHMENTS_PER_TURN, pickerExtensions } from "./attachments";
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
}: ComposerProps) {
  const { t } = useTranslation("agent");
  const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [dropActive, setDropActive] = useState(false);
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
  const activeModelId = modelId || (modelOptions[0] ? modelKey(modelOptions[0]) : "");
  const activeModel = modelOption(activeModelId, modelOptions);
  // Only offer/accept images when the active model advertises image input.
  // Unknown model (not in the catalog yet) → allow, to avoid over-restricting.
  const allowImages = activeModel ? activeModel.supportsImages !== false : true;
  const activeThinkingLevel = normalizeThinkingLevel(thinkingLevel);
  // Localized thinking-level label; unknown levels fall back to the raw value.
  const thinkingLevelLabel = (level: string) => t(`composer.thinkingLevelLabels.${level}`, { defaultValue: level });

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
    const next = [...attachments];
    const rejected: string[] = [];
    for (const { path, result } of classified) {
      if (next.some(attachment => attachment.path === path))
        continue;
      if (next.length >= MAX_ATTACHMENTS_PER_TURN) {
        rejected.push(t("composer.attachRejectedLimit", { name: fileNameFromPath(path), count: MAX_ATTACHMENTS_PER_TURN }));
        continue;
      }
      if (result.kind === null) {
        rejected.push(t("composer.attachRejectedReason", { name: fileNameFromPath(path), reason: result.reason }));
        continue;
      }
      if (result.kind === "image" && !allowImages) {
        rejected.push(t("composer.attachRejectedNoImage", { name: fileNameFromPath(path) }));
        continue;
      }
      next.push({ kind: result.kind, name: fileNameFromPath(path), path });
    }
    setAttachments(next);
    setAttachError(rejected.length > 0 ? t("composer.attachIgnored", { items: rejected.join("，") }) : null);
  }, [allowImages, attachments, t]);

  async function attachImageFiles(files: File[]) {
    // Save every file first, then attach in ONE addAttachmentPaths call:
    // calling it per file inside the loop reuses the same closure over the
    // pre-paste `attachments`, so each iteration's setAttachments overwrites
    // the previous one and only the last image survives.
    const saved: string[] = [];
    for (const file of files) {
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

    const selected = await open({
      filters: [{ extensions: pickerExtensions(allowImages), name: allowImages ? t("composer.attachDialogFilter") : t("composer.attachDialogFilterNoImage") }],
      multiple: true,
      title: t("composer.attachDialogTitle"),
    });
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    if (paths.length === 0)
      return;

    await addAttachmentPaths(paths);
  }

  function removeAttachment(path: string) {
    setAttachments(current => current.filter(attachment => attachment.path !== path));
  }

  useEffect(() => {
    if (disabled)
      return;

    let active = true;
    let dispose: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setDropActive(true);
        }
        else if (event.payload.type === "leave") {
          setDropActive(false);
        }
        else if (event.payload.type === "drop") {
          setDropActive(false);
          void addAttachmentPaths(event.payload.paths);
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
      setDropActive(false);
    };
  }, [addAttachmentPaths, disabled]);

  return (
    <form
      className={cn(
        "relative rounded-lg border border-line bg-surface/95 p-2 shadow-panel backdrop-blur",
        dropActive && "ring-2 ring-focus",
        className,
      )}
      onSubmit={handleSubmit}
    >
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
        onPasteImages={files => void attachImageFiles(files)}
      />
      {attachError
        ? <div className="px-1 pb-1 text-xs text-warning">{attachError}</div>
        : null}
      <div className="flex items-center justify-between pt-1">
        <div className="flex items-center gap-1">
          <button
            className="inline-flex size-7 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
            disabled={disabled || attachments.length >= MAX_ATTACHMENTS_PER_TURN}
            onClick={() => void handleAttachFiles()}
            type="button"
            aria-label={t("composer.attachFiles")}
            title={attachments.length >= MAX_ATTACHMENTS_PER_TURN ? t("composer.attachLimitReached") : allowImages ? t("composer.attachFilesHint") : t("composer.attachFilesHintNoImage")}
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
                      {tierIcon(approvalTier ?? "manual", "size-3 shrink-0")}
                      <span className="truncate">{t(`composer.approvalTier.${approvalTier ?? "manual"}`)}</span>
                      <ChevronDown className="size-3 shrink-0" />
                    </button>
                  )}
                >
                  {APPROVAL_TIERS.filter(tier => tier !== "sandbox" || isMacOS).map(tier => (
                    <SelectMenuItem
                      className="py-1.5"
                      key={tier}
                      selected={(approvalTier ?? "manual") === tier}
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
