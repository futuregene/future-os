import { Check, CircleAlert, Copy, GitBranch } from "lucide-react-native";
import { Fragment, memo, useState } from "react";
import { useTranslation } from "react-i18next";
import * as Clipboard from "expo-clipboard";
import { Pressable, Text, useWindowDimensions, View } from "react-native";
import { friendlyRunError, friendlyRunErrorTitle } from "../../../components/errorMessage";
import { MarkdownText } from "../../../components/MarkdownText";
import { canRecoverMessage } from "@future-os/remote-client/recovery";
import { useAppearance, useTheme } from "../../../theme/ThemeContext";
import { Button } from "../../../components/Button";
import { useStyles } from "./messageStyles";
import type { TimelineCardProps } from "./types";
import { RunIndicator, formatDuration } from "./messageUtils";
import { useCopyState } from "./useCopyState";
import { AttachmentChip } from "./AttachmentChip";
import { AttachmentImage } from "./AttachmentImage";
import { collapsedPromptHeight, isLongPrompt, promptPreview } from "./promptCollapse";
import { PromptReader } from "../components/PromptReader";
import { StatusDivider } from "./MessageStatus";
import { buildReplyBlocks, ProcessFold, ReplyBlockRow, splitFinalAnswer } from "./ReplyBlocks";

function TimelineCardView({
  item,
  isLatestAssistant,
  onOpenAttachment,
  onOpenFile,
  onOpenSession,
  onRetry,
  onContinue,
  onFork,
  onResolveToolTarget,
}: TimelineCardProps) {
  const styles = useStyles();
  const { colors, chat } = useTheme();
  // This phone's own reading preference, not the desktop's: the same choice
  // shapes every paired machine's conversation, because the transcript is drawn
  // here from the slices the desktop sends.
  const { replyDetail } = useAppearance();
  const { t, i18n } = useTranslation();
  const { copied, copy } = useCopyState();
  // The reader's own window: a collapsed prompt's cap is bounded by it (see
  // `promptCollapse`), and a rotation has to recompute that bound.
  const { height: windowHeight } = useWindowDimensions();
  // One bubble per row, and only a user's own prompt is ever readable in full;
  // the state lives here rather than in a wrapper component so the row keeps
  // its memoized identity.
  const [promptOpen, setPromptOpen] = useState(false);

  if (item.kind === "message") {
    if (item.role === "assistant") {
      // Single "time · N tokens" line, joined like the desktop MessageMeta
      // footer (which renders `parts.join(" · ")`).
      const outputTokens = item.outputTokens ?? 0;
      const usage =
        outputTokens > 0
          ? t("chat.tokens", {
            formattedCount: new Intl.NumberFormat(i18n.language).format(outputTokens),
          })
          : null;
      const footerStats = [item.durationMs != null ? formatDuration(item.durationMs) : null, usage]
        .filter((part): part is string => !!part)
        .join(" · ");
      // A compaction-only message is a standalone divider (the agent replaced
      // summarized history with a marker) — render just the hairline rule, no
      // copy footer, mirroring the desktop's MessageBlock special case.
      const dividerOnly =
        !item.streaming && item.segments?.length === 1 && item.segments[0]?.kind === "compaction";
      const terminationNotice = item.stopped
        ? t("failure.userStopped")
        : item.failed
          ? friendlyRunError(item.error, t)
          : item.truncated
            ? t("failure.modelResponseError")
            : null;
      const copyableText = item.text;
      // The reply's rendered slices.
      const blocks = buildReplyBlocks(item.segments ?? [], item.streaming);
      // This phone's reading setting: a reply that has settled keeps only the
      // answer it ends on, with the work that produced it folded into one row
      // above it. Nothing folds while the reply streams — the reader is
      // watching it happen — and a reply with no prose has no answer to
      // isolate, so it stays whole.
      const folded = replyDetail === "default" && !item.streaming && blocks.length > 0
        ? splitFinalAnswer(blocks)
        : null;
      const visibleBlocks = folded ? folded.answer : blocks;
      const hiddenWork = folded && folded.process.length > 0 ? folded.process : null;
      // The copy button follows what the row shows: with the work folded away it
      // belongs to the answer, not to notes the reader cannot see.
      const copyPayload = folded
        ? folded.answer.flatMap(block => (block.kind === "segment" && block.segment.kind === "text" ? [block.segment.text] : [])).join("\n\n")
        : copyableText;
      return (
        <View style={styles.assistantMessage}>
          {item.segments && item.segments.length > 0 ? (
            <View style={styles.segmentList}>
              {hiddenWork
                ? (
                    <ProcessFold
                      blocks={hiddenWork}
                      onOpenFile={onOpenFile}
                      onResolveToolTarget={onResolveToolTarget}
                    />
                  )
                : null}
              {visibleBlocks.map((block) => {
                const key = block.kind === "steps" ? block.segments[0]!.id : block.segment.id;
                return (
                  <Fragment key={key}>
                    <ReplyBlockRow
                      block={block}
                      onOpenFile={onOpenFile}
                      replyStreaming={item.streaming}
                      onResolveToolTarget={onResolveToolTarget}
                      streaming={item.streaming && block.kind === "segment" && block.segment === item.segments?.at(-1)}
                    />
                  </Fragment>
                );
              })}
            </View>
          ) : item.text.trim().length > 0 ? (
            <MarkdownText
              onOpenFile={onOpenFile}
              onOpenSession={onOpenSession}
              text={item.text}
              streaming={item.streaming}
            />
          ) : null}
          {terminationNotice ? (
            <View style={styles.terminationNotice}>
              <StatusDivider
                label={
                  item.stopped ? t("chat.responseStopped") : friendlyRunErrorTitle(item.error, t)
                }
              />
              {isLatestAssistant && !item.stopped ? (
                <Text
                  selectable
                  style={[styles.terminationNoticeText, item.stopped && styles.stoppedNoticeText]}
                >
                  {terminationNotice}
                </Text>
              ) : null}
            </View>
          ) : null}
          {/* Desktop parity: the retry/continue row sits right below the bubble
              content, above the copy/stats footer. It asks nothing about *why*
              the reply stopped — see `canRecoverMessage`. */}
          {canRecoverMessage(item, isLatestAssistant ? item.id : null) && (
            <View style={styles.recoveryRow}>
              <Button
                compact
                label={t("chat.retry")}
                onPress={() => onRetry?.(item)}
                variant="secondary"
              />
              <Button
                compact
                label={t("chat.continue")}
                onPress={() => onContinue?.(item)}
                variant="secondary"
              />
            </View>
          )}
          {dividerOnly ? null : item.streaming ? (
            // In-flight: the generating indicator occupies the same footer slot
            // the copy button uses once settled (desktop parity), so a streaming
            // reply never shows a copy button.
            <RunIndicator startedAt={item.startedAt} />
          ) : (
            <View style={styles.messageFooter}>
              {copyPayload.trim() ? (
                <Pressable
                  accessibilityLabel={t("chat.copyResponse")}
                  accessibilityRole="button"
                  hitSlop={8}
                  onPress={() => {
                    Clipboard.setStringAsync(copyPayload)
                      .then(() => copy())
                      .catch(() => { });
                  }}
                  style={styles.copyButton}
                >
                  {copied ? (
                    <Check color={colors.accent} size={15} />
                  ) : (
                    <Copy color={colors.inkMuted} size={15} />
                  )}
                </Pressable>
              ) : null}
              {onFork ? (
                <Pressable
                  accessibilityLabel={t("chat.fork")}
                  accessibilityRole="button"
                  hitSlop={8}
                  onPress={() => onFork(item)}
                  style={styles.copyButton}
                >
                  <GitBranch color={colors.inkMuted} size={15} />
                </Pressable>
              ) : null}
              <Text style={styles.messageDuration}>
                {footerStats || (item.stopped ? t("chat.responseStopped") : item.failed || item.truncated ? t("common.error") : t("chat.responseCompleted"))}
              </Text>
            </View>
          )}
        </View>
      );
    }
    // A prompt of goal-sized prose (a multi-kilobyte envelope, a pasted log) is
    // taller than the phone's viewport, and a row that tall is not drawable in
    // this transcript: measured on a reporting device, such a bubble painted its
    // accent fill and none of its text, at any scroll position, and re-opening
    // the app recomputed the same blank row. The failure follows the row's
    // height rather than the words — the same prompt renders when it is clipped
    // — so the transcript keeps a short preview and the control below opens the
    // whole prompt in a reader of its own (see `promptCollapse` and
    // `PromptReader`). Expanding in place cannot be offered: that is the shape
    // that fails.
    //
    // What keeps the row small is the head, not the cap: the preview is budgeted
    // in *rendered* lines (`PROMPT_PREVIEW_LINES`), because a prompt is one
    // `Text` node however much it wraps to, and a single node of ~630 dp is what
    // the device dropped. The markdown pipeline stays — the preview is the
    // prompt as the reader wrote it, headings and fences included — it just
    // carries a few lines of it.
    const longPrompt = isLongPrompt(item.text);
    const preview = longPrompt ? promptPreview(item.text) : { head: item.text, truncated: false };
    return (
      <View style={styles.userBlock}>
        {item.text.trim().length > 0 && (
          // The prompt renders through the same markdown pipeline as a reply (a
          // heading, list or code block the user typed now arrives formatted),
          // on the bubble's own tone because that fill is the accent colour.
          <View style={[
            styles.message,
            styles.userMessage,
            longPrompt
              ? [styles.promptCollapsed, { maxHeight: collapsedPromptHeight(chat.lineHeight, windowHeight) }]
              : null,
          ]}>
            <MarkdownText
              onOpenFile={onOpenFile}
              onOpenSession={onOpenSession}
              text={preview.head}
              tone="onAccent"
            />
          </View>
        )}
        {longPrompt && item.text.trim().length > 0 && (
          <Pressable
            accessibilityLabel={t("chat.expandPrompt")}
            accessibilityRole="button"
            onPress={() => setPromptOpen(true)}
            style={styles.promptToggle}
          >
            <Text style={styles.promptToggleLabel}>{t("chat.expandPrompt")}</Text>
          </Pressable>
        )}
        {item.attachments && item.attachments.length > 0 && (
          <View style={styles.attachmentRow}>
            {item.attachments.map(attachment => (
              // A picture is drawn as itself (and opens the viewer); everything
              // else stays a named chip.
              attachment.kind === "image"
                ? (
                    <AttachmentImage
                      attachment={attachment}
                      key={`${item.id}:${attachment.path}`}
                      onOpen={onOpenAttachment ? () => onOpenAttachment(attachment) : undefined}
                    />
                  )
                : (
                    <AttachmentChip
                      attachment={attachment}
                      key={`${item.id}:${attachment.path}`}
                      onOpen={onOpenAttachment ? () => onOpenAttachment(attachment) : undefined}
                    />
                  )
            ))}
          </View>
        )}
        {longPrompt && (
          <PromptReader
            onClose={() => setPromptOpen(false)}
            text={item.text}
            visible={promptOpen}
          />
        )}
      </View>
    );
  }

  if (item.kind === "notice") {
    const warning = item.tone === "warning";
    // Danger notices carry raw agent/relay error blobs — surface the friendly
    // classification, never the developer-oriented dump.
    const noticeText =
      item.text === "truncated"
        ? t("chat.truncated")
        : warning
          ? item.text
          : friendlyRunError(item.text, t);
    return (
      <View style={[styles.notice, warning ? styles.warningNotice : styles.dangerNotice]}>
        <CircleAlert color={warning ? colors.warning : colors.danger} size={17} />
        <Text style={[styles.noticeText, { color: warning ? colors.warning : colors.danger }]}>
          {noticeText}
        </Text>
      </View>
    );
  }

  // Approval items are rendered docked above the composer (see ChatScreen),
  // never inline in the transcript — so there is nothing to draw here.
  return null;
}

// Long conversations update the trailing streaming item frequently. Keep
// already-settled rows out of those renders; FlatList virtualization limits
// how many are mounted, while memoization limits work inside the mounted
// window.
export const TimelineCard = memo(TimelineCardView);
