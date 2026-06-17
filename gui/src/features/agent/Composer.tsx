import type { ChangeEvent, FormEvent, KeyboardEvent } from "react";
import type { AgentModelOption } from "../../integrations/agent/models";
import type { ReferenceTargetSearchResult } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./types";
import { open } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, ArrowUp, Beaker, Box, Check, ChevronDown, FileDiff, Microscope, Paperclip, PlayCircle, Sparkles, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { modelLabel } from "../../integrations/agent/models";
import { searchReferenceTargets } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { fileNameFromPath } from "./attachments";

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
  placeholder,
  textareaClassName,
  workspaceId,
}: ComposerProps) {
  const [value, setValue] = useState("");
  const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [caretPosition, setCaretPosition] = useState(0);
  const [referenceResults, setReferenceResults] = useState<ReferenceTargetSearchResult[]>([]);
  const [referenceSearchOpen, setReferenceSearchOpen] = useState(false);
  const [selectedReferenceIndex, setSelectedReferenceIndex] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const activeModelId = modelId || modelOptions[0]?.id || "";
  const activeMention = useMemo(() => findActiveMention(value, caretPosition), [caretPosition, value]);
  const modelMenuRef = useDismissableLayer<HTMLDivElement>({
    enabled: modelMenuOpen,
    onDismiss: () => setModelMenuOpen(false),
  });

  useEffect(() => {
    let cancelled = false;

    async function loadReferenceResults() {
      if (!workspaceId || !activeMention || disabled) {
        setReferenceResults([]);
        setReferenceSearchOpen(false);
        return;
      }

      try {
        const results = await searchReferenceTargets({
          limit: 8,
          query: activeMention.query,
          workspaceId,
        });
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

    void loadReferenceResults();

    return () => {
      cancelled = true;
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
    setReferenceSearchOpen(false);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
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
        insertReference(referenceResults[selectedReferenceIndex]);
        return;
      }
    }

    if (event.key === "Escape" && referenceSearchOpen) {
      event.preventDefault();
      setReferenceSearchOpen(false);
      return;
    }

    if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing)
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

  function insertReference(reference: ReferenceTargetSearchResult) {
    if (!activeMention)
      return;

    const label = escapeMarkdownLinkLabel(`${reference.targetType}:${reference.title}`);
    const targetId = encodeFutureReferenceId(reference.targetId);
    const markdown = `[${label}](futureos://${reference.targetType}/${targetId})`;
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

  async function handleAttachFiles() {
    if (disabled)
      return;

    const selected = await open({
      multiple: true,
      title: "Attach files",
    });
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
    if (paths.length === 0)
      return;

    setAttachments(current => [
      ...current,
      ...paths
        .filter(path => !current.some(attachment => attachment.path === path))
        .map(path => ({
          name: fileNameFromPath(path),
          path,
        })),
    ]);
  }

  function removeAttachment(path: string) {
    setAttachments(current => current.filter(attachment => attachment.path !== path));
  }

  return (
    <form
      className={cn(
        "relative rounded-lg border border-line bg-white/95 p-2 shadow-panel backdrop-blur",
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
      <textarea
        ref={textareaRef}
        className={cn(
          "h-14 w-full resize-none border-0 bg-transparent px-2 py-1 text-sm leading-5 text-ink outline-none placeholder:text-ink-muted",
          textareaClassName,
        )}
        placeholder={placeholder ?? "Ask FutureOS to plan, research, analyze, edit, or prepare a workflow..."}
        value={value}
        disabled={disabled}
        onKeyDown={handleKeyDown}
        onChange={handleChange}
        onClick={updateCaret}
        onKeyUp={updateCaret}
        onSelect={updateCaret}
      />
      {attachments.length > 0
        ? (
            <div className="flex flex-wrap gap-1.5 px-1 pb-1">
              {attachments.map(attachment => (
                <span
                  className="inline-flex max-w-64 items-center gap-1.5 rounded-md bg-surface-subtle px-2 py-1 text-xs text-ink-soft"
                  key={attachment.path}
                  title={attachment.path}
                >
                  <Paperclip className="size-3 shrink-0" />
                  <span className="truncate">{attachment.name}</span>
                  <button
                    aria-label={`Remove ${attachment.name}`}
                    className="inline-flex size-4 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-white hover:text-ink"
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
      <div className="flex items-center justify-between pt-1">
        <div className="flex items-center gap-1">
          <button
            className="inline-flex size-7 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
            disabled={disabled}
            onClick={() => void handleAttachFiles()}
            type="button"
            aria-label="Attach context"
            title="Attach context"
          >
            <Paperclip className="size-3.5" />
          </button>
          <button
            className="inline-flex size-7 items-center justify-center rounded-md text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
            type="button"
            aria-label="Agent presets"
            title="Agent presets"
          >
            <Sparkles className="size-3.5" />
          </button>
        </div>
        <div className="flex items-center gap-2">
          <div className="relative hidden md:block" ref={modelMenuRef}>
            <button
              className="inline-flex h-7 max-w-48 items-center gap-1.5 rounded-md bg-surface-subtle px-2 text-xs font-medium text-ink-soft transition-colors hover:bg-surface hover:text-ink"
              onClick={() => setModelMenuOpen(open => !open)}
              type="button"
              title="Model"
            >
              <span className="truncate">{modelLabel(activeModelId, modelOptions)}</span>
              <ChevronDown className="size-3 shrink-0" />
            </button>
            {modelMenuOpen
              ? (
                  <div className="absolute bottom-9 right-0 z-30 w-56 rounded-lg border border-line-soft bg-white p-1 shadow-panel">
                    {modelOptions.length === 0
                      ? (
                          <div className="px-2 py-2 text-sm text-ink-muted">Start Future Agent to load models.</div>
                        )
                      : null}
                    {modelOptions.map(model => (
                      <button
                        className="flex h-9 w-full items-center gap-2 rounded-md px-2 text-left text-sm transition-colors hover:bg-surface-subtle"
                        key={`${model.provider}/${model.id}`}
                        onClick={() => {
                          onModelChange?.(model.id);
                          setModelMenuOpen(false);
                        }}
                        type="button"
                      >
                        <span className="min-w-0 flex-1">
                          <span className="block truncate font-medium text-ink">{model.label}</span>
                          <span className="block truncate text-xs text-ink-muted">{model.provider}</span>
                        </span>
                        {activeModelId === model.id ? <Check className="size-4 text-ink-soft" /> : null}
                      </button>
                    ))}
                  </div>
                )
              : null}
          </div>
          <button
            className="inline-flex size-7 items-center justify-center rounded-md bg-accent text-white transition-colors hover:bg-blue-700 disabled:bg-blue-200"
            disabled={(!value.trim() && attachments.length === 0) || disabled}
            type="submit"
            aria-label="Send"
            title="Send"
          >
            <ArrowUp className="size-3.5" />
          </button>
        </div>
      </div>
    </form>
  );
}

function ReferenceSearchMenu({
  onSelect,
  results,
  selectedIndex,
}: {
  onSelect: (reference: ReferenceTargetSearchResult) => void;
  results: ReferenceTargetSearchResult[];
  selectedIndex: number;
}) {
  return (
    <div className="absolute bottom-full left-2 z-30 mb-2 w-[min(30rem,calc(100%-1rem))] rounded-lg border border-line-soft bg-white p-1 shadow-panel">
      {results.length === 0
        ? <div className="px-2 py-2 text-sm text-ink-muted">No references found.</div>
        : null}
      {results.map((result, index) => (
        <button
          className={cn(
            "flex h-11 w-full items-center gap-2 rounded-md px-2 text-left transition-colors",
            index === selectedIndex ? "bg-surface-subtle" : "hover:bg-surface-subtle",
          )}
          key={`${result.targetType}:${result.targetId}`}
          onMouseDown={(event) => {
            event.preventDefault();
            onSelect(result);
          }}
          type="button"
        >
          {referenceIcon(result.targetType)}
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium text-ink">{result.title}</span>
            <span className="block truncate text-xs text-ink-muted">
              {result.targetType}
              {result.subtitle ? ` · ${result.subtitle}` : ""}
            </span>
          </span>
        </button>
      ))}
    </div>
  );
}

function referenceIcon(targetType: string) {
  const className = "size-4 shrink-0 text-ink-soft";
  switch (targetType) {
    case "approval":
      return <AlertTriangle className={className} />;
    case "research":
      return <Microscope className={className} />;
    case "review":
      return <FileDiff className={className} />;
    case "run":
      return <PlayCircle className={className} />;
    case "tool":
      return <Beaker className={className} />;
    default:
      return <Box className={className} />;
  }
}

function findActiveMention(value: string, caretPosition: number) {
  const beforeCaret = value.slice(0, caretPosition);
  const match = beforeCaret.match(/(^|\s)@([^\s@]*)$/);
  if (!match)
    return null;

  const markerOffset = match[1].length;
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

function encodeFutureReferenceId(value: string) {
  return encodeURIComponent(value).replace(/[!'()*]/g, character => `%${character.charCodeAt(0).toString(16).toUpperCase()}`);
}
