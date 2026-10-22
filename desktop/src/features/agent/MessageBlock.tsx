import type { AgentMessage } from "@future-os/thread-projection";
import type { ActivityTargetResolver } from "./AgentActivityList";
import type { ReplyBlock, StepSegment } from "./replyBlocks";
import { GitBranch, RotateCcw, StepForward } from "lucide-react";
import { Fragment, memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "../../components/ui/CopyButton";
import { useCopyState } from "../../components/ui/useCopyState";
import { cn } from "../../lib/cn";
import { foldsSettledReply, useReplyDetail } from "../../lib/replyDisplay";
import { STREAMED_BLOCK_CONTAINMENT } from "../markdown/LiveMarkdownContext";
import { MarkdownContent, StreamingMarkdownContent } from "../markdown/MarkdownContent";
import { AgentActivityLine, AgentActivityList } from "./AgentActivityList";
import { canContinueResponse, isCompletedWithoutReply } from "./agentMessageFormatters";
import { MessageMeta } from "./MessageMeta";
import { AttachmentChip } from "./messages/AttachmentChip";
import { MessageFrame } from "./messages/MessageFrame";
import { CompactionDivider, StatusDivider } from "./messages/MessageStatus";
import { buildReplyBlocks, replyTailBlock, splitFinalAnswer } from "./replyBlocks";
import { ReplyProcess } from "./ReplyProcess";
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
  // Reading preference: the `default` level keeps a settled reply to the answer
  // it ends on, with the work that produced it folded into one row above it.
  // Expansion is the reader's, per reply — each row owns its own.
  const finalAnswerOnly = foldsSettledReply(useReplyDetail());
  const [processExpanded, setProcessExpanded] = useState(false);
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
  // The reply's rendered slices, and the one ↑/↓ lands on when the ladder stops
  // at this turn's reply: its last slice that still has output. A reply opens
  // with the work it ran, so the row's own top would put reasoning at the
  // reading edge and the answer below it — the rung names this block instead,
  // and the row marks it so a jump can measure it (`useQuestionNav`).
  const blocks = segments ? buildReplyBlocks(segments, streaming) : null;
  // Fold the work only once the reply settles: while it streams the reader is
  // watching it happen, and every live block stays on screen. A reply with no
  // prose of its own has no answer to isolate, so nothing folds.
  const folded = blocks && finalAnswerOnly && !streaming ? splitFinalAnswer(blocks) : null;
  const visibleBlocks = folded ? folded.answer : blocks;
  const hiddenWork = folded && folded.process.length > 0 ? folded.process : null;
  const hiddenSteps = hiddenWork ? replySteps(hiddenWork) : [];
  const hiddenTools = hiddenSteps.reduce((total, segment) => segment.kind === "activity" ? total + (segment.item.count ?? 1) : total, 0);
  const hiddenThoughts = hiddenSteps.filter(segment => segment.kind === "thinking").length;
  const tailBlock = visibleBlocks ? replyTailBlock(visibleBlocks) : null;
  // The reply's final slice, whatever kind it is — what "this reply is still
  // being written" means for a reasoning block.
  const lastSegmentId = segments ? (segments[segments.length - 1]?.id ?? null) : null;
  // Plain-text payload for the copy button: joined text slices when the reply is
  // segmented, otherwise the raw content. Activity lines are excluded. While the
  // reading setting folds the work away, the button belongs to the answer it is
  // shown beside, so it copies that answer and not prose that came before it.
  const copyableText = (folded
    ? proseText(folded.answer)
    : segments
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

  // One rendered slice of the reply; `tail` is the index of the block the ↑/↓
  // ladder lands on, measured over whatever the row currently shows.
  const renderBlocks = (list: readonly ReplyBlock[], tail: number | null) => list.map((block, index) => {
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
    return index === tail
      ? <div data-reply-tail="" key={key}>{body}</div>
      : <Fragment key={key}>{body}</Fragment>;
  });

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
      {segments && visibleBlocks
        ? (
            <div className="space-y-3">
              {hiddenWork
                ? (
                    <ReplyProcess
                      expanded={processExpanded}
                      thoughts={hiddenThoughts}
                      tools={hiddenTools}
                      onToggle={() => setProcessExpanded(value => !value)}
                    >
                      {renderBlocks(hiddenWork, null)}
                    </ReplyProcess>
                  )
                : null}
              {renderBlocks(visibleBlocks, tailBlock)}
            </div>
          )
        : message.content
          ? isUser
            // A prompt is markdown like a reply: the same pipeline renders the
            // user's `@` file mentions (their `[name](./path)` link form), `#`
            // conversation references and `[label](http…)` links — and a
            // heading, list or code block they typed arrives formatted instead
            // of as its own source. A prompt never streams, so it takes the
            // static parse rather than the streaming one.
            ? <MarkdownContent content={message.content} workspaceId={workspaceId} />
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

/** The prose a set of rendered blocks carries, joined the way the copy button wants it. */
function proseText(blocks: readonly ReplyBlock[]): string {
  return blocks
    .flatMap(block => (block.kind === "segment" && block.segment.kind === "text" ? [block.segment.text] : []))
    .join("\n\n");
}

/** Every step (reasoning or a settled call) a set of rendered blocks holds. */
function replySteps(blocks: readonly ReplyBlock[]): StepSegment[] {
  return blocks.flatMap((block) => {
    if (block.kind === "steps")
      return block.segments;
    return block.segment.kind === "activity" || block.segment.kind === "thinking" ? [block.segment] : [];
  });
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
