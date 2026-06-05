import type { AgentMessage } from "./types";
import { Paperclip } from "lucide-react";
import { cn } from "../../lib/cn";
import { formatTime } from "../../lib/date";
import { AgentActivityList } from "./AgentActivityList";
import { MarkdownContent } from "./MarkdownContent";
import { PlanBlock } from "./PlanBlock";

interface MessageBlockProps {
  message: AgentMessage;
}

export function MessageBlock({ message }: MessageBlockProps) {
  const isUser = message.role === "user";

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
              : <MarkdownContent content={message.content} />
            : null}
          {message.attachments && message.attachments.length > 0
            ? (
                <div className={cn("mt-2 flex flex-wrap gap-1.5", isUser && "justify-end")}>
                  {message.attachments.map(attachment => (
                    <span
                      className="inline-flex max-w-72 items-center gap-1.5 rounded-md bg-white px-2 py-1 text-xs text-ink-soft ring-1 ring-line-soft"
                      key={`${message.id}:${attachment.path}`}
                      title={attachment.path}
                    >
                      <Paperclip className="size-3 shrink-0" />
                      <span className="truncate">{attachment.name}</span>
                    </span>
                  ))}
                </div>
              )
            : null}
          {message.plan ? <PlanBlock steps={message.plan} /> : null}
          {!isUser ? <AgentActivityList items={message.activityItems} /> : null}
        </div>
      </div>
    </article>
  );
}
