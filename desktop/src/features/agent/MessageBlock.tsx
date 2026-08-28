import type { AgentMessage, MessageAttachment } from "@future-os/thread-projection";
import { convertFileSrc } from "@tauri-apps/api/core";
import { FileText, GitBranch, Paperclip, RotateCcw, StepForward } from "lucide-react";
import { Fragment, memo, useState } from "react";
import { useTranslation } from "react-i18next";
import { CopyButton } from "../../components/ui/CopyButton";
import { useCopyState } from "../../components/ui/useCopyState";
import { openPath } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { formatDateTime, formatMessageTimestamp } from "../../lib/date";
import { formatNumber } from "../../lib/format";
import { emitFutureEvent } from "../../lib/futureEvents";
import { useNow } from "../../lib/useNow";
import { FilePreviewOverlay } from "../filepreview/FilePreviewOverlay";
import { previewKindForPath } from "../filepreview/previewKind";
import { StreamingMarkdownContent } from "../markdown/MarkdownContent";
import { SafeLink } from "../markdown/renderers/SafeLink";
import { AgentActivityLine, AgentActivityList } from "./AgentActivityList";
import { splitExternalLinkSegments } from "./externalLinks";
import { parseMentionSegments } from "./mentionMarkdown";
import { MessageMeta } from "./MessageMeta";
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
  /** Show the model's reasoning block (driven by the "show thinking" setting). */
  showThinking?: boolean;
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
  showThinking,
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
        <div className={cn("mb-1 flex items-center gap-2", isUser && "justify-end")}>
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
          )}
        >
          {segments
            ? (
                <div className="space-y-3">
                  {segments.map((segment) => {
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
                      // Reasoning stays in timeline order; hidden unless the
                      // "show thinking" setting is on.
                      return showThinking
                        ? (
                            <ThinkingBlock
                              key={segment.id}
                              live={segment.id === liveSegmentId}
                              text={segment.text}
                              workspaceId={workspaceId}
                            />
                          )
                        : null;
                    }
                    if (segment.kind === "compaction") {
                      return (
                        <CompactionDivider
                          error={segment.error}
                          key={segment.id}
                          status={segment.status}
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
          {!isUser && message.terminationNotice
            ? (
                <div className="mt-4">
                  <StatusDivider
                    label={message.stopped ? t("thread.responseStopped") : (message.terminationTitle ?? t("thread.responseIncomplete"))}
                  />
                  {isLast === true && !message.stopped
                    ? (
                        <p className={cn("mt-2 text-sm leading-6", message.stopped ? "text-ink-muted" : "text-ink-soft")}>
                          {message.terminationNotice}
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
          {!streaming && !isUser && onFork
            ? (
                <button
                  className={cn(
                    "rounded p-1 text-ink-muted hover:text-ink will-change-[opacity] transition-opacity duration-200",
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
          {!isUser ? <MessageMeta message={message} visible={hovered} /> : null}
          {streaming && !isUser && !showThinking && message.thinkingActive
            ? <span className="select-none text-xs text-ink-muted">{t("message.thinking")}</span>
            : null}
        </div>
      </div>
    </article>
  );
}

/**
 * User messages render as plain text (never markdown — the user's `*`/`#`/`1.`
 * stay literal), except `@` file mentions, which show in the accent color like
 * the composer pill, and `[label](http…)` links (e.g. the coach prompt's manual
 * link), which render clickable via SafeLink. Everything else is verbatim.
 */
function UserMessageText({ content }: { content: string }) {
  const segments = parseMentionSegments(content);

  return (
    <p className="whitespace-pre-wrap">
      {segments.map(segment =>
        segment.mention
          ? <span key={segment.key} className="font-medium text-accent">{segment.text}</span>
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

function StatusDivider({ label, failed = false, pulsing = false, title }: {
  label: string;
  failed?: boolean;
  pulsing?: boolean;
  title?: string;
}) {
  return (
    <div
      aria-label={label}
      className={cn("flex select-none items-center gap-3 py-1", pulsing && "animate-pulse")}
      role={failed ? "alert" : "status"}
      title={title}
    >
      <span className={cn("h-px flex-1", failed ? "bg-danger/40" : "bg-line")} />
      <span className={cn("whitespace-nowrap text-xs", failed ? "text-danger" : "text-ink-muted")}>{label}</span>
      <span className={cn("h-px flex-1", failed ? "bg-danger/40" : "bg-line")} />
    </div>
  );
}

/** Inline divider marking where the agent auto-compacted the conversation. */
function CompactionDivider({
  tokensBefore,
  status = "completed",
  error,
  trigger,
}: {
  tokensBefore?: number;
  status?: "running" | "completed" | "failed";
  error?: string;
  trigger?: string;
}) {
  const { t, i18n } = useTranslation("agent");
  const manual = trigger === "manual";
  const label = status === "running"
    ? manual ? t("message.manuallyCompacting") : t("message.compacting")
    : status === "failed"
      ? manual ? t("message.manualCompactionFailed") : t("message.compactionFailed")
      : manual
        ? t("message.manuallyCompacted")
        : tokensBefore && tokensBefore > 0
          ? t("message.compactedTokens", {
              formattedCount: formatNumber(tokensBefore, i18n.language),
            })
          : t("message.compacted");
  const failed = status === "failed";
  return <StatusDivider failed={failed} label={label} pulsing={status === "running"} title={error} />;
}

/**
 * Live "generating" marker shown in place of the copy button while a reply
 * streams: a small amber dot with a pulsing ping halo (no brain icon — the
 * motion is the signal). `label` is exposed to assistive tech only.
 */
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
  // A thumbnail (images now; PDF page previews later) renders as a small preview.
  // If it's absent (generation failed, or the thread's image dir was reclaimed)
  // or fails to load, fall back to the named pill instead of a blank box.
  const { t } = useTranslation("agent");
  const [failed, setFailed] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  const previewKind = previewKindForPath(attachment.path);
  const missingMessage = t("attachment.fileMissing", { name: attachment.name });

  function handleOpen() {
    if (previewKind) {
      setPreviewOpen(true);
      return;
    }
    void openPath(attachment.path).catch(() => {
      emitFutureEvent("toast", { message: missingMessage, tone: "error" });
    });
  }

  if (attachment.thumbnail && !failed) {
    return (
      <>
        <button
          className="inline-flex items-center overflow-hidden rounded-md ring-1 ring-line-soft transition-shadow hover:ring-line"
          onClick={handleOpen}
          title={attachment.name}
          type="button"
        >
          <img
            alt={attachment.name}
            className="size-16 object-cover"
            onError={() => setFailed(true)}
            src={convertFileSrc(attachment.thumbnail)}
          />
        </button>
        {/* Preview the full-size original. If it's gone (moved/reclaimed), toast
            that it's damaged and close — the 96px thumbnail isn't worth previewing. */}
        <FilePreviewOverlay
          kind={previewKind ?? "image"}
          name={attachment.name}
          onClose={() => setPreviewOpen(false)}
          open={previewOpen}
          path={attachment.path}
          unavailableMessage={missingMessage}
        />
      </>
    );
  }
  return (
    <>
      <button
        className="inline-flex max-w-72 items-center gap-1.5 rounded-md bg-surface px-2 py-1 text-xs text-ink-soft ring-1 ring-line-soft transition-colors hover:bg-surface-subtle hover:text-ink"
        onClick={handleOpen}
        title={attachment.path}
        type="button"
      >
        {attachment.kind === "file"
          ? <FileText className="size-3 shrink-0" />
          : <Paperclip className="size-3 shrink-0" />}
        <span className="truncate">{attachment.name}</span>
      </button>
      {previewKind
        ? (
            <FilePreviewOverlay
              kind={previewKind}
              name={attachment.name}
              onClose={() => setPreviewOpen(false)}
              open={previewOpen}
              path={attachment.path}
              unavailableMessage={missingMessage}
            />
          )
        : null}
    </>
  );
}
