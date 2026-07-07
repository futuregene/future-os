import type { ChangeEvent, ClipboardEvent, FormEvent, KeyboardEvent } from "react";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { WorkspaceFileResult } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./agentThreadTypes";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowUp, ChevronDown, FileText, Paperclip, ShieldCheck, Square, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SelectMenu, SelectMenuItem } from "../../components/ui/SelectMenu";
import { modelKey, modelLabel, modelOption, normalizeThinkingLevel, thinkingLevels } from "../../integrations/agent/agentClient";
import { useProviderNames } from "../../integrations/agent/useProviderNames";
import { savePastedImage, searchWorkspaceFiles } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { isMacOS } from "../../lib/platform";
import { classifyAttachment, fileNameFromPath, imageExtensionFromMime, MAX_ATTACHMENTS_PER_TURN, pickerExtensions } from "./attachments";

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
  const [value, setValue] = useState("");
  const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [dropActive, setDropActive] = useState(false);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [thinkingMenuOpen, setThinkingMenuOpen] = useState(false);
  const [approvalMenuOpen, setApprovalMenuOpen] = useState(false);
  const providerNames = useProviderNames();
  const [caretPosition, setCaretPosition] = useState(0);
  const [referenceResults, setReferenceResults] = useState<WorkspaceFileResult[]>([]);
  const [referenceSearchOpen, setReferenceSearchOpen] = useState(false);
  const [selectedReferenceIndex, setSelectedReferenceIndex] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  // IME composition guard. WebKit (Tauri's macOS webview) doesn't reliably set
  // `event.isComposing` on the Enter that commits a composition, so we track it
  // ourselves: the committing keydown fires while this ref is still true.
  const isComposingRef = useRef(false);
  const activeModelId = modelId || (modelOptions[0] ? modelKey(modelOptions[0]) : "");
  const activeModel = modelOption(activeModelId, modelOptions);
  // Only offer/accept images when the active model advertises image input.
  // Unknown model (not in the catalog yet) → allow, to avoid over-restricting.
  const allowImages = activeModel ? activeModel.supportsImages !== false : true;
  const activeThinkingLevel = normalizeThinkingLevel(thinkingLevel);
  const activeMention = useMemo(() => findActiveMention(value, caretPosition), [caretPosition, value]);

  useEffect(() => {
    // Hand-rolled cancel guard (not useAsyncResource): this effect drives several
    // states (results + open flag) with an early-return branch, which doesn't map
    // onto the primitive's single-resource shape. See gui/CLAUDE.md §4.
    let cancelled = false;

    if (!workspaceId || !activeMention || disabled) {
      setReferenceResults([]);
      setReferenceSearchOpen(false);
      return;
    }

    async function loadFileResults(query: string, wsId: string) {
      try {
        const results = await searchWorkspaceFiles({ limit: 20, query, workspaceId: wsId });
        if (!cancelled) {
          setReferenceResults(results);
          setReferenceSearchOpen(true);
          setSelectedReferenceIndex(0);
        }
      }
      catch {
        if (!cancelled) {
          setReferenceResults([]);
          setReferenceSearchOpen(false);
        }
      }
    }

    // Debounce so a fast typist doesn't walk the workspace on every keystroke.
    const timer = setTimeout(() => void loadFileResults(activeMention.query ?? "", workspaceId), 120);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [activeMention, disabled, workspaceId]);

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    submitValue();
  }

  function submitValue() {
    const trimmed = value.trim();
    if ((!trimmed && attachments.length === 0) || disabled)
      return;
    onSend({ attachments, content: trimmed });
    setValue("");
    setAttachments([]);
    setAttachError(null);
    setReferenceSearchOpen(false);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    // While an IME composition is active, let the input method consume every key
    // (Enter commits the composition, arrows move through candidates) — never run
    // our own Enter/Tab/arrow shortcuts. `keyCode === 229` is the legacy sentinel
    // for a keystroke routed to the IME, covering webviews that leave
    // `isComposing` unset on the committing keydown.
    if (event.nativeEvent.isComposing || isComposingRef.current || event.nativeEvent.keyCode === 229)
      return;

    if (referenceSearchOpen && referenceResults.length > 0) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setSelectedReferenceIndex(index => (index + 1) % referenceResults.length);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setSelectedReferenceIndex(index => (index - 1 + referenceResults.length) % referenceResults.length);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        const selected = referenceResults[selectedReferenceIndex];
        if (selected)
          insertReference(selected);
        return;
      }
    }

    if (event.key === "Escape" && referenceSearchOpen) {
      event.preventDefault();
      setReferenceSearchOpen(false);
      return;
    }

    if (event.key !== "Enter" || event.shiftKey)
      return;

    event.preventDefault();
    submitValue();
  }

  function handleChange(event: ChangeEvent<HTMLTextAreaElement>) {
    setValue(event.target.value);
    setCaretPosition(event.target.selectionStart);
  }

  function updateCaret() {
    setCaretPosition(textareaRef.current?.selectionStart ?? value.length);
  }

  function insertReference(file: WorkspaceFileResult) {
    if (!activeMention)
      return;

    // Plain relative-path markdown link (not futureos://): the agent runs with
    // the workspace as cwd, so `./path` is a real path it can read directly.
    // Angle-bracket the target only when the path has spaces, which would
    // otherwise break the markdown link.
    const label = escapeMarkdownLinkLabel(file.path);
    const target = `./${file.path}`;
    const markdown = `[${label}](${/\s/.test(target) ? `<${target}>` : target})`;
    const nextValue = `${value.slice(0, activeMention.start)}${markdown}${value.slice(activeMention.end)}`;
    const nextCaret = activeMention.start + markdown.length;
    setValue(nextValue);
    setCaretPosition(nextCaret);
    setReferenceSearchOpen(false);
    window.requestAnimationFrame(() => {
      textareaRef.current?.focus();
      textareaRef.current?.setSelectionRange(nextCaret, nextCaret);
    });
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

  async function handlePaste(event: ClipboardEvent<HTMLTextAreaElement>) {
    if (disabled)
      return;

    const imageItems = Array.from(event.clipboardData?.items ?? []).filter(
      item => item.kind === "file" && item.type.startsWith("image/"),
    );
    if (imageItems.length === 0)
      return;

    event.preventDefault();
    for (const item of imageItems) {
      const file = item.getAsFile();
      if (!file)
        continue;

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
      {referenceSearchOpen && activeMention
        ? (
            <ReferenceSearchMenu
              results={referenceResults}
              selectedIndex={selectedReferenceIndex}
              onSelect={insertReference}
            />
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
      <textarea
        ref={textareaRef}
        className={cn(
          "h-14 w-full resize-none border-0 bg-transparent px-2 py-1 text-sm leading-5 text-ink outline-none placeholder:text-ink-muted",
          textareaClassName,
        )}
        placeholder={placeholder ?? t("composer.placeholder")}
        value={value}
        disabled={disabled}
        onKeyDown={handleKeyDown}
        onChange={handleChange}
        onCompositionStart={() => { isComposingRef.current = true; }}
        onCompositionEnd={() => { isComposingRef.current = false; }}
        onPaste={handlePaste}
        onClick={updateCaret}
        onKeyUp={updateCaret}
        onSelect={updateCaret}
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
                  disabled={(!value.trim() && attachments.length === 0) || disabled}
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

function ReferenceSearchMenu({
  onSelect,
  results,
  selectedIndex,
}: {
  onSelect: (file: WorkspaceFileResult) => void;
  results: WorkspaceFileResult[];
  selectedIndex: number;
}) {
  const { t } = useTranslation("agent");
  return (
    <div className="absolute bottom-full left-2 z-30 mb-2 w-[min(30rem,calc(100%-1rem))] rounded-lg border border-line-soft bg-surface p-1 shadow-panel">
      {results.length === 0
        ? <div className="px-2 py-2 text-sm text-ink-muted">{t("composer.noFiles")}</div>
        : null}
      {results.map((file, index) => {
        // Split the path so the directory reads muted and the filename strong.
        const dir = file.path.slice(0, file.path.length - file.name.length);
        return (
          <button
            className={cn(
              "flex h-9 w-full items-center gap-2 rounded-md px-2 text-left transition-colors",
              index === selectedIndex ? "bg-surface-subtle" : "hover:bg-surface-subtle",
            )}
            key={file.path}
            onMouseDown={(event) => {
              event.preventDefault();
              onSelect(file);
            }}
            type="button"
          >
            <FileText className="size-4 shrink-0 text-ink-soft" />
            <span className="min-w-0 flex-1 truncate text-sm">
              {dir ? <span className="text-ink-muted">{dir}</span> : null}
              <span className="font-medium text-ink">{file.name}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}

function findActiveMention(value: string, caretPosition: number) {
  const beforeCaret = value.slice(0, caretPosition);
  const match = beforeCaret.match(/(^|\s)@([^\s@]*)$/);
  if (!match)
    return null;

  // Group 1 `(^|\s)` is a required capture — present whenever `match` is.
  const markerOffset = match[1]!.length;
  const start = caretPosition - match[0].length + markerOffset;
  return {
    end: caretPosition,
    query: match[2],
    start,
  };
}

function escapeMarkdownLinkLabel(value: string) {
  return value
    .replace(/\s+/g, " ")
    .trim()
    .replace(/\\/g, "/")
    .replace(/\[/g, "(")
    .replace(/\]/g, ")");
}
