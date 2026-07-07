import type { FormEvent } from "react";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { MessageAttachment } from "./agentThreadTypes";
import type { MentionEditorHandle } from "./MentionEditor";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowUp, ChevronDown, Paperclip, ShieldCheck, Square, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
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

export interface ComposerSendPayload {
  attachments: MessageAttachment[];
  content: string;
}

interface ComposerProps {
  onSend: (payload: ComposerSendPayload) => void;
  className?: string;
  disabled?: boolean;
  modelId?: string;
  modelOptions: AgentModelOption[];
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
  const editorRef = useRef<MentionEditorHandle | null>(null);
  const activeModelId = modelId || (modelOptions[0] ? modelKey(modelOptions[0]) : "");
  const activeModel = modelOption(activeModelId, modelOptions);
  // Only offer/accept images when the active model advertises image input.
  // Unknown model (not in the catalog yet) → allow, to avoid over-restricting.
  const allowImages = activeModel ? activeModel.supportsImages !== false : true;
  const activeThinkingLevel = normalizeThinkingLevel(thinkingLevel);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    submitValue();
  }

  function submitValue() {
    const trimmed = (editorRef.current?.getContent() ?? "").trim();
    if ((!trimmed && attachments.length === 0) || disabled)
      return;
    onSend({ attachments, content: trimmed });
    editorRef.current?.clear();
    setAttachments([]);
    setAttachError(null);
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
    for (const file of files) {
      try {
        const buffer = await file.arrayBuffer();
        const saved = await savePastedImage({
          bytes: Array.from(new Uint8Array(buffer)),
          extension: imageExtensionFromMime(file.type) ?? "png",
        });
        await addAttachmentPaths([saved.path]);
      }
      catch {
        // Ignore a single failed paste; other clipboard items still attach.
      }
    }
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
                  panelClassName="w-44 overflow-hidden"
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
                      <ShieldCheck className="size-3 shrink-0" />
                      <span className="truncate">{t(`composer.approvalTier.${approvalTier ?? "manual"}`)}</span>
                      <ChevronDown className="size-3 shrink-0" />
                    </button>
                  )}
                >
                  {APPROVAL_TIERS.filter(tier => tier !== "sandbox" || isMacOS).map(tier => (
                    <SelectMenuItem
                      key={tier}
                      selected={(approvalTier ?? "manual") === tier}
                      onSelect={() => {
                        onChangeApprovalTier(tier);
                        setApprovalMenuOpen(false);
                      }}
                    >
                      <span className="min-w-0 flex-1 truncate font-medium text-ink">{t(`composer.approvalTier.${tier}`)}</span>
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
                <span className="truncate">{modelLabel(activeModelId, modelOptions)}</span>
                <ChevronDown className="size-3 shrink-0" />
              </button>
            )}
          >
            {modelOptions.length === 0
              ? (
                  <div className="px-3 py-2 text-sm text-ink-muted">{t("composer.startAgentForModels")}</div>
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
                  disabled={(inputEmpty && attachments.length === 0) || disabled}
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

function thinkingLevelLabel(level: string) {
  switch (level) {
    case "off":
      return "Off";
    case "minimal":
      return "Minimal";
    case "low":
      return "Low";
    case "medium":
      return "Medium";
    case "high":
      return "High";
    case "xhigh":
      return "XHigh";
    default:
      return level;
  }
}
