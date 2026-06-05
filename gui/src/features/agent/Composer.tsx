import type { FormEvent, KeyboardEvent } from "react";
import type { MessageAttachment } from "./types";
import { open } from "@tauri-apps/plugin-dialog";
import { ArrowUp, Check, ChevronDown, Paperclip, Sparkles, X } from "lucide-react";
import { useState } from "react";
import { agentModelOptions, modelLabel } from "../../integrations/agent/models";
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
  onModelChange?: (modelId: string) => void;
  placeholder?: string;
  textareaClassName?: string;
}

export function Composer({
  onSend,
  className,
  disabled,
  modelId,
  onModelChange,
  placeholder,
  textareaClassName,
}: ComposerProps) {
  const [value, setValue] = useState("");
  const [attachments, setAttachments] = useState<MessageAttachment[]>([]);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const activeModelId = modelId ?? agentModelOptions[0].id;
  const modelMenuRef = useDismissableLayer<HTMLDivElement>({
    enabled: modelMenuOpen,
    onDismiss: () => setModelMenuOpen(false),
  });

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
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key !== "Enter" || event.shiftKey || event.nativeEvent.isComposing)
      return;

    event.preventDefault();
    submitValue();
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
        "rounded-lg border border-line bg-white/95 p-2 shadow-panel backdrop-blur",
        className,
      )}
      onSubmit={handleSubmit}
    >
      <textarea
        className={cn(
          "h-14 w-full resize-none border-0 bg-transparent px-2 py-1 text-sm leading-5 text-ink outline-none placeholder:text-ink-muted",
          textareaClassName,
        )}
        placeholder={placeholder ?? "Ask FutureOS to plan, research, analyze, edit, or prepare a workflow..."}
        value={value}
        disabled={disabled}
        onKeyDown={handleKeyDown}
        onChange={event => setValue(event.target.value)}
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
              <span className="truncate">{modelLabel(activeModelId)}</span>
              <ChevronDown className="size-3 shrink-0" />
            </button>
            {modelMenuOpen
              ? (
                  <div className="absolute bottom-9 right-0 z-30 w-56 rounded-lg border border-line-soft bg-white p-1 shadow-panel">
                    {agentModelOptions.map(model => (
                      <button
                        className="flex h-9 w-full items-center gap-2 rounded-md px-2 text-left text-sm transition-colors hover:bg-surface-subtle"
                        key={model.id}
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
