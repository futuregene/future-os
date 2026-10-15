import type { AgentMessage } from "@future-os/thread-projection";
import type { ActivityTargetResolver } from "./AgentActivityList";
import type { ReplyBlock } from "./replyBlocks";
import { GitBranch, RotateCcw, StepForward } from "lucide-react";
import { Fragment, memo } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "../../components/ui/CopyButton";
import { useCopyState } from "../../components/ui/useCopyState";
import { cn } from "../../lib/cn";
import { STREAMED_BLOCK_CONTAINMENT } from "../markdown/LiveMarkdownContext";
import { StreamingMarkdownContent } from "../markdown/MarkdownContent";
import { SafeLink } from "../markdown/renderers/SafeLink";
import { AgentActivityLine, AgentActivityList } from "./AgentActivityList";
import { canContinueResponse, isCompletedWithoutReply } from "./agentMessageFormatters";
import { splitExternalLinkSegments } from "./externalLinks";
import { parseMentionSegments } from "./mentionMarkdown";
import { MessageMeta } from "./MessageMeta";
import { AttachmentChip } from "./messages/AttachmentChip";
import { MessageFrame } from "./messages/MessageFrame";
import { CompactionDivider, StatusDivider } from "./messages/MessageStatus";
import { buildReplyBlocks, replyTailBlock } from "./replyBlocks";
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
  /**
   * Where a tool row's target comes from when it arrived without one.
   *
   * Only a remote transcript has this: a live local run carries its calls'
   * arguments, and the lean history page this app reads from another machine
   * does not, so those rows ask for the one they are missing when a reader opens
   * them (see `ActivityTargetResolver`). Unset everywhere else.
   */
  resolveActivityTarget?: ActivityTargetResolver;
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
  resolveActivityTarget,
  workspaceId,
  workspacePath,
}: MessageBlockProps) {
  const { t } = useTranslation("agent");
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
  // Built once and placed by role: the assistant's sits in the reply's own
  // footer, the user's goes to `MessageFrame`'s footer, outside their bubble.
  const copyControl = copyableText
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
    : null;

  // The reply's rendered slices, and the one ↑/↓ lands on when the ladder stops
  // at this turn's reply: its last slice that still has output. A reply opens
  // with the work it ran, so the row's own top would put reasoning at the
  // reading edge and the answer below it — the rung names this block instead,
  // and the row marks it so a jump can measure it (`useQuestionNav`).
  const blocks = segments ? buildReplyBlocks(segments, streaming) : null;
  const tailBlock = blocks ? replyTailBlock(blocks) : null;
  // The reply's final slice, whatever kind it is — what "this reply is still
  // being written" means for a reasoning block.
  const lastSegmentId = segments ? (segments[segments.length - 1]?.id ?? null) : null;

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
    <MessageFrame
      author={t(message.authorKey)}
      bodyClassName={streaming ? `**:data-[streamed-block]:${STREAMED_BLOCK_CONTAINMENT}` : undefined}
      createdAt={message.createdAt}
      dataMessageId={dataMessageId}
      footer={isUser ? copyControl : undefined}
      onEnter={() => onHover(message.id)}
      onLeave={() => onLeave(message.id)}
      role={isUser ? "user" : "assistant"}
    >
      {segments && blocks
        ? (
            <div className="space-y-3">
              {blocks.map((block, index) => {
                const key = blockKey(block);
                const body = (
                  <ReplyBlockBody
                    block={block}
                    lastSegmentId={lastSegmentId}
                    liveSegmentId={liveSegmentId}
                    message={message}
                    resolveActivityTarget={resolveActivityTarget}
                    streaming={streaming}
                    workspaceId={workspaceId}
                    workspacePath={workspacePath}
                  />
                );
                // The block the ladder's reply-tail rung lands on is wrapped so
                // its own top can be measured inside this row: the wrapper
                // carries no style, so it reports where the block sits without
                // moving it (`[data-reply-tail]`, read by `useQuestionNav`).
                return index === tailBlock
                  ? <div data-reply-tail="" key={key}>{body}</div>
                  : <Fragment key={key}>{body}</Fragment>;
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
        ? (
            <AgentActivityList
              items={message.activityItems}
              resolveTarget={resolveActivityTarget}
              runId={message.runId}
              workspacePath={workspacePath}
            />
          )
        : !isUser && !segments
            ? (
                <AgentActivityList
                  items={message.activityItems}
                  resolveTarget={resolveActivityTarget}
                  workspacePath={workspacePath}
                />
              )
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
      {!isUser
        ? (
            <div className="flex flex-wrap items-center gap-2 mt-3 justify-start">
              {streaming
                ? (
                    <StreamingIndicator
                      label={message.reconnecting
                        ? t("message.reconnecting", { attempt: message.reconnecting.attempt, maxRetries: message.reconnecting.maxRetries })
                        : t("message.generating")}
                      showLabel={Boolean(message.reconnecting)}
                    />
                  )
                : copyControl}
              {!streaming && onFork
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
              <MessageMeta message={message} hovered={hovered} />
              {streaming && message.thinkingActive && !segments?.some(segment => segment.kind === "thinking")
                ? <span className="select-none text-xs text-ink-muted">{t("message.thinking")}</span>
                : null}
            </div>
          )
        : null}
    </MessageFrame>
  );
}

/**
 * A stable key for a rendered reply block. A reply carries many same-kind
 * slices, so the key is the segment id the shared projection assigned — never
 * the kind.
 */
function blockKey(block: ReplyBlock): string {
  return block.kind === "steps" ? block.segments[0]!.id : block.segment.id;
}

/**
 * One rendered slice of a reply: folded steps, prose, reasoning, a compaction
 * divider or an activity line.
 *
 * Extracted from the row's map so the one block the ↑/↓ ladder lands on can be
 * wrapped (and therefore measured) without a second copy of this switch — the
 * ladder's reply-tail rung names a slice, and the wrapper is how the jump finds
 * where that slice starts.
 */
function ReplyBlockBody({
  block,
  lastSegmentId,
  liveSegmentId,
  message,
  resolveActivityTarget,
  streaming,
  workspaceId,
  workspacePath,
}: {
  block: ReplyBlock;
  /** The reply's final slice, whatever it is: what "still streaming" means. */
  lastSegmentId: string | null;
  /** The slice a live text block is being written into, or null when settled. */
  liveSegmentId: string | null;
  message: AgentMessage;
  /** Where a tool row's target comes from when it arrived without one. */
  resolveActivityTarget?: ActivityTargetResolver;
  streaming: boolean;
  workspaceId?: string | null;
  workspacePath?: string | null;
}) {
  if (block.kind === "steps") {
    return (
      <ReplySteps
        resolveTarget={resolveActivityTarget}
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
        live={segment.id === liveSegmentId}
        workspaceId={workspaceId}
      />
    );
  }
  if (segment.kind === "thinking") {
    return (
      <ThinkingBlock
        live={streaming && segment.id === lastSegmentId}
        text={segment.text}
        workspaceId={workspaceId}
      />
    );
  }
  if (segment.kind === "compaction") {
    return (
      <CompactionDivider
        error={segment.error}
        status={segment.status}
        tokensAfter={segment.tokensAfter}
        tokensBefore={segment.tokensBefore}
        trigger={segment.trigger}
      />
    );
  }
  return (
    <AgentActivityLine
      item={segment.item}
      resolveTarget={resolveActivityTarget}
      runId={message.runId}
      workspacePath={workspacePath}
    />
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
