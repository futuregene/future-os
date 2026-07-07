import type { AgentMessage, MessageAttachment } from "./agentThreadTypes";
import { convertFileSrc } from "@tauri-apps/api/core";
import { FileText, Paperclip, RotateCcw, StepForward } from "lucide-react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "../../components/ui/CopyButton";
import { useCopyState } from "../../components/ui/useCopyState";
import { cn } from "../../lib/cn";
import { formatTime } from "../../lib/date";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { AgentActivityLine, AgentActivityList } from "./AgentActivityList";
import { MessageMeta } from "./MessageMeta";
import { ThinkingBlock } from "./ThinkingBlock";

interface MessageBlockProps {
  message: AgentMessage;
  /** Whether this row is the hovered one (single-owner state in MessageList). */
  hovered: boolean;
  /** Whether this is the last message in the thread. */
  isLast?: boolean;
  recoverySource?: AgentMessage | null;
  /** Show the model's reasoning block (driven by the "show thinking" setting). */
  showThinking?: boolean;
  onContinue?: (message: AgentMessage) => void;
  onHover: (id: string) => void;
  onLeave: (id: string) => void;
  onRetry?: (message: AgentMessage, source: AgentMessage) => void;
  workspaceId?: string | null;
}

export function MessageBlock({
  message,
  hovered,
  isLast,
  recoverySource,
  showThinking,
  onContinue,
  onHover,
  onLeave,
  onRetry,
  workspaceId,
}: MessageBlockProps) {
  const { t } = useTranslation("agent");
  const { copiedKey, copy } = useCopyState();
  const isUser = message.role === "user";
  // While the reply streams, the footer is pinned open and shows a live activity
  // indicator instead of the copy button; the copy button returns once it settles.
  const streaming = !isUser && message.status === "streaming";
  // Retry/Continue only make sense on the latest turn — once a newer round has
  // started, recovering an earlier failed turn would fork the conversation.
  const canRecover = !isUser && message.status === "failed" && isLast === true;
  // A local narrowed to the non-empty segment array (or null) so the render can
  // map over it without a non-null assertion.
  const segments = !isUser && message.segments && message.segments.length > 0 ? message.segments : null;
  // Plain-text payload for the copy button: joined text slices when the reply is
  // segmented, otherwise the raw content. Activity lines are excluded.
  const copyableText = (segments
    ? segments.flatMap(segment => (segment.kind === "text" ? [segment.text] : [])).join("\n\n")
    : message.content ?? "").trim();

  return (
    <article className="flex justify-center">
      <div
        className="min-w-0 w-full max-w-3xl"
        onPointerLeave={() => onLeave(message.id)}
        onPointerOver={() => onHover(message.id)}
      >
        <div className={cn("mb-1 flex items-center gap-2", isUser && "justify-end")}>
          <span className="text-sm font-semibold text-ink">
            {message.authorKey ? t(message.authorKey) : message.author}
          </span>
          <span className="text-xs text-ink-muted">{formatTime(message.createdAt)}</span>
        </div>
        <div
          className={cn(
            "text-sm leading-6 text-ink",
            isUser
              ? "ml-auto w-fit max-w-2xl wrap-break-word rounded-lg bg-surface-subtle px-4 py-3 text-left"
              : "w-full",
          )}
        >
          {segments
            ? (
                <div className="space-y-3">
                  {segments.map((segment) => {
                    if (segment.kind === "text") {
                      return (
                        <MarkdownContent
                          content={segment.text}
                          key={segment.id}
                          workspaceId={workspaceId}
                        />
                      );
                    }
                    if (segment.kind === "thinking") {
                      // Reasoning stays in timeline order; hidden unless the
                      // "show thinking" setting is on.
                      return showThinking
                        ? (
                            <ThinkingBlock
                              key={segment.id}
                              text={segment.text}
                              workspaceId={workspaceId}
                            />
                          )
                        : null;
                    }
                    return <AgentActivityLine item={segment.item} key={segment.id} />;
                  })}
                </div>
              )
            : message.content
              ? isUser
                ? <UserMessageText content={message.content} />
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
          {!isUser && !segments ? <AgentActivityList items={message.activityItems} /> : null}
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
                          {t("message.retry")}
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
                          {t("message.continue")}
                        </button>
                      )
                    : null}
                </div>
              )
            : null}
        </div>
        <div className={cn("flex items-center gap-2", isUser ? "mt-1 justify-end" : "mt-3")}>
          {streaming
            ? <StreamingIndicator label={t("message.generating")} />
            : copyableText
              ? (
                  <CopyButton
                  // `will-change-[opacity]` keeps the button on its own compositor
                  // layer at all times: WKWebView (tauri#12800 family) drops repaints
                  // of in-flow content until a window resize, so hide/show — and the
                  // fade, which is only safe because the compositor animates a
                  // promoted layer's opacity — must never depend on a repaint. Do not
                  // remove the will-change without re-testing stale-paint ghosts.
                    className={cn(
                      "will-change-[opacity] transition-opacity duration-200",
                      hovered ? "opacity-100" : "pointer-events-none opacity-0",
                    )}
                    copied={copiedKey === "default"}
                    onCopy={() => void copy(copyableText)}
                  />
                )
              : null}
          {!isUser ? <MessageMeta message={message} visible={hovered} /> : null}
          {streaming && !isUser && !showThinking && message.thinkingActive
            ? <span className="select-none text-xs text-ink-muted">{t("message.thinking")}</span>
            : null}
          {!isUser && message.stopped
            ? <span className="select-none text-xs text-ink-muted">{t("message.stopped")}</span>
            : null}
        </div>
      </div>
    </article>
  );
}

/**
 * Live "generating" marker shown in place of the copy button while a reply
 * streams: a small amber dot with a pulsing ping halo (no brain icon — the
 * motion is the signal). `label` is exposed to assistive tech only.
 */
/**
 * A composer `@` mention serializes to `[name](./path)` (or `[name](<./path>)`
 * when the path has spaces). Matches those file links so the user bubble can
 * show the mention name in the accent color — matching the composer pill.
 */
const MENTION_LINK = /\[([^\]]+)\]\((?:<(\.\/[^>]+)>|(\.\/[^)\s]+))\)/g;

/**
 * User messages render as plain text (never markdown — the user's `*`/`#`/`1.`
 * stay literal), except `@` file mentions, which show in the accent color like
 * the composer pill. Everything else is verbatim.
 */
function UserMessageText({ content }: { content: string }) {
  // `key` is the segment's character offset — stable and unique within the text.
  const segments: { text: string; mention: boolean; key: number }[] = [];
  let last = 0;
  MENTION_LINK.lastIndex = 0;
  for (let match = MENTION_LINK.exec(content); match; match = MENTION_LINK.exec(content)) {
    if (match.index > last)
      segments.push({ text: content.slice(last, match.index), mention: false, key: last });
    segments.push({ text: match[1] ?? "", mention: true, key: match.index });
    last = match.index + match[0].length;
  }
  if (last < content.length)
    segments.push({ text: content.slice(last), mention: false, key: last });

  return (
    <p className="whitespace-pre-wrap">
      {segments.map(segment =>
        segment.mention
          ? <span key={segment.key} className="font-medium text-accent">{segment.text}</span>
          : <span key={segment.key}>{segment.text}</span>,
      )}
    </p>
  );
}

function StreamingIndicator({ label }: { label: string }) {
  return (
    <div aria-label={label} className="flex items-center px-1 py-1.5" role="status">
      <span className="relative flex size-2">
        <span className="absolute inline-flex size-full animate-ping rounded-full bg-generating opacity-75" />
        <span className="relative inline-flex size-2 rounded-full bg-generating" />
      </span>
    </div>
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
