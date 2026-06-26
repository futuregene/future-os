import type { AgentMessage, MessageAttachment } from "./agentThreadTypes";
import { convertFileSrc } from "@tauri-apps/api/core";
import { FileText, Paperclip, RotateCcw, StepForward } from "lucide-react";
import { cn } from "../../lib/cn";
import { formatTime } from "../../lib/date";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { AgentActivityList } from "./AgentActivityList";
import { PlanBlock } from "./PlanBlock";

interface MessageBlockProps {
  message: AgentMessage;
  recoverySource?: AgentMessage | null;
  onContinue?: (message: AgentMessage) => void;
  onRetry?: (message: AgentMessage, source: AgentMessage) => void;
  workspaceId?: string | null;
}

export function MessageBlock({
  message,
  recoverySource,
  onContinue,
  onRetry,
  workspaceId,
}: MessageBlockProps) {
  const isUser = message.role === "user";
  const canRecover = !isUser && message.status === "failed";

  return (
    <article className="flex justify-center">
      <div className="min-w-0 w-full max-w-3xl">
        <div className={cn("mb-1 flex items-center gap-2", isUser && "justify-end")}>
          <span className="text-sm font-semibold text-ink">{message.author}</span>
          <span className="text-xs text-ink-muted">{formatTime(message.createdAt)}</span>
        </div>
        <div
          className={cn(
            "text-sm leading-6 text-ink",
            isUser
              ? "ml-auto w-fit max-w-2xl break-words rounded-lg bg-surface-subtle px-4 py-3 text-right"
              : "w-full",
          )}
        >
          {message.content
            ? isUser
              ? <p className="whitespace-pre-wrap">{message.content}</p>
              : <MarkdownContent content={message.content} workspaceId={workspaceId} />
            : null}
          {message.attachments && message.attachments.length > 0
            ? (
                <div className={cn("mt-2 flex flex-wrap gap-1.5", isUser && "justify-end")}>
                  {message.attachments.map(attachment => (
                    <AttachmentChip key={`${message.id}:${attachment.path}`} attachment={attachment} />
                  ))}
                </div>
              )
            : null}
          {message.plan ? <PlanBlock steps={message.plan} /> : null}
          {!isUser ? <AgentActivityList items={message.activityItems} /> : null}
          {canRecover
            ? (
                <div className="mt-3 flex flex-wrap gap-2">
                  {recoverySource && onRetry
                    ? (
                        <button
                          className="inline-flex h-8 items-center gap-1.5 rounded-md border border-line bg-surface px-2.5 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                          onClick={() => onRetry(message, recoverySource)}
                          type="button"
                        >
                          <RotateCcw className="size-3.5" />
                          Retry
                        </button>
                      )
                    : null}
                  {onContinue
                    ? (
                        <button
                          className="inline-flex h-8 items-center gap-1.5 rounded-md border border-line bg-surface px-2.5 text-xs font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
                          onClick={() => onContinue(message)}
                          type="button"
                        >
                          <StepForward className="size-3.5" />
                          Continue
                        </button>
                      )
                    : null}
                </div>
              )
            : null}
        </div>
      </div>
    </article>
  );
}

function AttachmentChip({ attachment }: { attachment: MessageAttachment }) {
  const thumbSrc = attachment.thumbnail ?? (attachment.kind === "image" ? attachment.path : null);
  if (attachment.kind === "image" && thumbSrc) {
    return (
      <span
        className="inline-flex items-center overflow-hidden rounded-md ring-1 ring-line-soft"
        title={attachment.name}
      >
        <img
          alt={attachment.name}
          className="size-16 object-cover"
          src={convertFileSrc(thumbSrc)}
        />
      </span>
    );
  }
  return (
    <span
      className="inline-flex max-w-72 items-center gap-1.5 rounded-md bg-surface px-2 py-1 text-xs text-ink-soft ring-1 ring-line-soft"
      title={attachment.path}
    >
      {attachment.kind === "pdf" || attachment.kind === "text"
        ? <FileText className="size-3 shrink-0" />
        : <Paperclip className="size-3 shrink-0" />}
      <span className="truncate">{attachment.name}</span>
    </span>
  );
}
