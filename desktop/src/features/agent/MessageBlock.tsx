import type { AgentMessage } from "@future-os/thread-projection";
import { GitBranch, RotateCcw, StepForward } from "lucide-react";
import { Fragment, memo } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "../../components/ui/CopyButton";
import { useCopyState } from "../../components/ui/useCopyState";
import { cn } from "../../lib/cn";
import { formatDateTime, formatMessageTimestamp } from "../../lib/date";
import { useNow } from "../../lib/useNow";
import { STREAMED_BLOCK_CONTAINMENT } from "../markdown/LiveMarkdownContext";
import { StreamingMarkdownContent } from "../markdown/MarkdownContent";
import { SafeLink } from "../markdown/renderers/SafeLink";
import { AgentActivityLine, AgentActivityList } from "./AgentActivityList";
import { canContinueResponse, isCompletedWithoutReply } from "./agentMessageFormatters";
import { splitExternalLinkSegments } from "./externalLinks";
import { parseMentionSegments } from "./mentionMarkdown";
import { MessageMeta } from "./MessageMeta";
import { AttachmentChip } from "./messages/AttachmentChip";
import { CompactionDivider, StatusDivider } from "./messages/MessageStatus";
import { buildReplyBlocks } from "./replyBlocks";
import { ReplySteps } from "./ReplySteps";
import { ThinkingBlock } from "./ThinkingBlock";

interface MessageBlockProps {
  message: AgentMessage;
  /** Whether this row is the hovered one (single-owner state in MessageList). */
  hovered: boolean;
  /** Anchor for scroll restoration when an older page loads above this row. */
  dataMessageId?: string;
  /** Whether this is the last message in the thread. */
  isLast?: boolean;
  recoverySource?: AgentMessage | null;
  onContinue?: (message: AgentMessage) => void;
  onFork?: (message: AgentMessage) => void;
  onHover: (id: string) => void;
  onLeave: (id: string) => void;
  onRetry?: (message: AgentMessage, source: AgentMessage) => void;
  workspaceId?: string | null;
  workspacePath?: string | null;
}

// Memoized: pushed streaming deltas re-render MessageList frequently and any hover
// change re-renders it too, but each row's props (message reference kept stable
// by patchMessage, stable callbacks) are unchanged for all but the affected rows,
// so a shallow-prop comparison skips re-rendering — and re-running their nested
// MarkdownContent — for every settled message.
export const MessageBlock = memo(MessageBlockImpl);

function MessageBlockImpl({
  message,
  hovered,
  dataMessageId,
  isLast,
  recoverySource,
  onContinue,
  onFork,
  onHover,
  onLeave,
  onRetry,
  workspaceId,
  workspacePath,
}: MessageBlockProps) {
  const { i18n, t } = useTranslation("agent");
  // Re-render on a minute cadence so the relative timestamp ("3 minutes ago")
  // stays accurate as it ages, instead of freezing at its first-render value.
  const now = useNow();
  const { copiedKey, copy } = useCopyState();
  const isUser = message.role === "user";
  // While the reply streams, the footer is pinned open and shows a live activity
  // indicator instead of the copy button; the copy button returns once it settles.
  const streaming = !isUser && message.status === "streaming";
  const noFinalReply = isCompletedWithoutReply(message);
  // Retry/Continue only make sense on the latest exchange — once a newer round has
  // started, recovering an earlier failed exchange would fork the conversation.
  // Also suppress for interrupted runs (the agent may still be processing the
  // original after a GUI restart — retrying would race the in-flight run).
  const canRecover = !isUser
    && message.status === "failed"
    && isLast === true
    && !message.stopped;
  // A local narrowed to the non-empty segment array (or null) so the render can
  // map over it without a non-null assertion.
  const segments = !isUser && message.segments && message.segments.length > 0 ? message.segments : null;
  // While streaming, only the LAST text/thinking segment is still growing —
  // mark it `live` so its code blocks skip re-highlighting on every delta
  // poll tick (O(block) per tick → O(n²) over the reply). Closed segments
  // keep full rendering. Once the run settles, `streaming` flips off and the
  // tail re-renders fully highlighted.
  const liveSegmentId = streaming && segments
    ? (() => {
        for (let index = segments.length - 1; index >= 0; index--) {
          const segment = segments[index]!;
          if (segment.kind === "text" || segment.kind === "thinking")
            return segment.id;
        }
        return null;
      })()
    : null;
  // Plain-text payload for the copy button: joined text slices when the reply is
  // segmented, otherwise the raw content. Activity lines are excluded.
  const copyableText = (segments
    ? segments.flatMap(segment => (segment.kind === "text" ? [segment.text] : [])).join("\n\n")
    : message.content ?? "").trim();

  // A reloaded compaction marker is a message carrying only a compaction
  // segment: render just the divider (no author header / bubble), matching the
  // inline divider the live path shows mid-reply.
  if (segments && segments.length === 1 && segments[0]!.kind === "compaction") {
    return (
      <article className="flex justify-center">
        <div className="min-w-0 w-full max-w-3xl" data-message-id={dataMessageId}>
          <CompactionDivider
            error={segments[0]!.error}
            status={segments[0]!.status}
            tokensAfter={segments[0]!.tokensAfter}
            tokensBefore={segments[0]!.tokensBefore}
            trigger={segments[0]!.trigger}
          />
        </div>
      </article>
    );
  }

  return (
    <article className="flex justify-center">
      <div
        className="min-w-0 w-full max-w-3xl"
        data-message-id={dataMessageId}
        onPointerLeave={() => onLeave(message.id)}
        onPointerOver={() => onHover(message.id)}
      >
        <div className={cn(
          "flex items-center gap-2",
          isUser ? "mb-1 justify-end" : "mb-3 border-b border-line-soft/40 pb-2",
        )}
        >
          <span className="text-sm font-semibold text-ink">
            {t(message.authorKey)}
          </span>
          <span className="text-xs text-ink-muted" title={formatDateTime(message.createdAt, i18n.language)}>
            {formatMessageTimestamp(message.createdAt, i18n.language, {
              now,
              justNowLabel: t("message.justNow"),
            })}
          </span>
        </div>
        <div
          className={cn(
            "text-sm leading-6 text-ink",
            isUser
              ? "ml-auto w-fit max-w-2xl wrap-break-word rounded-lg bg-surface-subtle px-4 py-3 text-left"
              : "w-full",
            // See LiveMarkdownContext for the full mechanism. While the reply
            // streams, every pushed delta mutates the growing tail and the
            // browser re-computes preferred widths and re-lays-out the message
            // from scratch — visible in WebContent samples as the dominant
            // frame-time cost (`RenderBlock::layout`, `computePreferredLogical
            // Widths`, `paintObject`, `performFlexLayout`). `contain` on the
            // live (still-growing) segment keeps that work inside the tail
            // instead of re-walking every settled text/thinking/code block —
            // the difference between a chat that keeps up and one that freezes
            // on a long reasoning reply.
            streaming ? `**:data-[streamed-block]:${STREAMED_BLOCK_CONTAINMENT}` : "",
          )}
        >
          {segments
            ? (
                <div className="space-y-3">
                  {buildReplyBlocks(segments, streaming).map((block) => {
                    if (block.kind === "steps") {
                      return (
                        <ReplySteps
                          key={block.segments[0]!.id}
                          segments={block.segments}
                          workspaceId={workspaceId}
                          workspacePath={workspacePath}
                          runId={message.runId}
                        />
                      );
                    }
                    const segment = block.segment;
                    if (segment.kind === "text") {
                      return (
                        <StreamingMarkdownContent
                          content={segment.text}
                          key={segment.id}
                          live={segment.id === liveSegmentId}
                          workspaceId={workspaceId}
                        />
                      );
                    }
                    if (segment.kind === "thinking") {
                      return (
                        <ThinkingBlock
                          key={segment.id}
                          live={streaming && segment === segments[segments.length - 1]}
                          text={segment.text}
                          workspaceId={workspaceId}
                        />
                      );
                    }
                    if (segment.kind === "compaction") {
                      return (
                        <CompactionDivider
                          error={segment.error}
                          key={segment.id}
                          status={segment.status}
                          tokensAfter={segment.tokensAfter}
                          tokensBefore={segment.tokensBefore}
                          trigger={segment.trigger}
                        />
                      );
                    }
                    return <AgentActivityLine item={segment.item} key={segment.id} workspacePath={workspacePath} runId={message.runId} />;
                  })}
                </div>
              )
            : message.content
              ? isUser
                ? <UserMessageText content={message.content} />
                : (
                    <StreamingMarkdownContent
                      content={message.content}
                      workspaceId={workspaceId}
                      live={streaming}
                    />
                  )
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
          {!isUser && !segments && message.runId
            ? <AgentActivityList items={message.activityItems} workspacePath={workspacePath} runId={message.runId} />
            : !isUser && !segments
                ? <AgentActivityList items={message.activityItems} workspacePath={workspacePath} />
                : null}
          {!isUser && (message.terminationNotice || noFinalReply)
            ? (
                <div className="mt-4">
                  <StatusDivider
                    label={noFinalReply ? t("thread.noFinalReply") : message.stopped ? t("thread.responseStopped") : (message.terminationTitle ?? t("thread.responseIncomplete"))}
                    warning={!message.stopped && !noFinalReply}
                  />
                  {isLast === true && !message.stopped
                    ? (
                        <p className={cn("mt-2 text-sm leading-6", message.stopped ? "text-ink-muted" : "text-ink-soft")}>
                          {noFinalReply ? t("thread.noFinalReplyDetail") : message.terminationNotice}
                        </p>
                      )
                    : null}
                </div>
              )
            : null}
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
                  {onContinue && canContinueResponse(message)
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
        <div className={cn("flex flex-wrap items-center gap-2", isUser ? "mt-1 justify-end" : "mt-3 justify-start")}>
          {streaming
            ? (
                <StreamingIndicator
                  label={message.reconnecting
                    ? t("message.reconnecting", { attempt: message.reconnecting.attempt, maxRetries: message.reconnecting.maxRetries })
                    : t("message.generating")}
                  showLabel={Boolean(message.reconnecting)}
                />
              )
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
                      "will-change-[opacity] transition-opacity duration-200 focus-visible:pointer-events-auto focus-visible:opacity-100",
                      hovered ? "opacity-100" : "pointer-events-none opacity-0",
                    )}
                    copied={copiedKey === "default"}
                    onCopy={() => void copy(copyableText)}
                  />
                )
              : null}
          {!streaming && !isUser && onFork
            ? (
                <button
                  className={cn(
                    "rounded p-1 text-ink-muted hover:text-ink will-change-[opacity] transition-opacity duration-200 focus-visible:pointer-events-auto focus-visible:opacity-100",
                    hovered ? "opacity-100" : "pointer-events-none opacity-0",
                  )}
                  onClick={() => onFork(message)}
                  title={t("message.fork")}
                  type="button"
                >
                  <GitBranch className="size-3.5" />
                </button>
              )
            : null}
          {!isUser ? <MessageMeta message={message} hovered={hovered} /> : null}
          {streaming && !isUser && message.thinkingActive && !segments?.some(segment => segment.kind === "thinking")
            ? <span className="select-none text-xs text-ink-muted">{t("message.thinking")}</span>
            : null}
        </div>
      </div>
    </article>
  );
}

/**
 * User messages render as plain text (never markdown — the user's `*`/`#`/`1.`
 * stay literal), except `@` file mentions and `#` session references, which show
 * in the accent color like the composer pill, and `[label](http…)` links (e.g.
 * the coach prompt's manual link), which render clickable via SafeLink.
 * Everything else is verbatim.
 */
function UserMessageText({ content }: { content: string }) {
  const segments = parseMentionSegments(content);

  return (
    <p className="whitespace-pre-wrap">
      {segments.map(segment =>
        segment.mention
          ? (
              <span key={segment.key} className="font-medium text-accent">
                {/* A session reference keeps the composer's `#` marker, so a
                    reference to another conversation cannot be mistaken for a
                    file mention in a glance over the transcript. */}
                {segment.sessionId ? `#${segment.text}` : segment.text}
              </span>
            )
          : (
              <Fragment key={segment.key}>
                {splitExternalLinkSegments(segment.text).map(linkSegment =>
                  linkSegment.link
                    ? (
                        <SafeLink href={linkSegment.href ?? ""} key={linkSegment.key}>
                          {linkSegment.text}
                        </SafeLink>
                      )
                    : <span key={linkSegment.key}>{linkSegment.text}</span>,
                )}
              </Fragment>
            ),
      )}
    </p>
  );
}

/**
 * Live "generating" marker shown in place of the copy button while a reply
 * streams: a small amber dot with a pulsing ping halo (no brain icon — the
 * motion is the signal). Reconnect status also gets a visible label.
 */
function StreamingIndicator({ label, showLabel }: { label: string; showLabel: boolean }) {
  return (
    <div aria-label={label} className="flex items-center gap-2 px-1 py-1.5" role="status">
      <span className="relative flex size-2">
        <span className="absolute inline-flex size-full animate-ping rounded-full bg-generating opacity-75" />
        <span className="relative inline-flex size-2 rounded-full bg-generating" />
      </span>
      {showLabel ? <span className="text-xs text-ink-muted">{label}</span> : null}
    </div>
  );
}
