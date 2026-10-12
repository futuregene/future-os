import { Check, CircleAlert, Copy, GitBranch } from "lucide-react-native";
import { memo } from "react";
import { useTranslation } from "react-i18next";
import * as Clipboard from "expo-clipboard";
import { Pressable, Text, View } from "react-native";
import { friendlyRunError, friendlyRunErrorTitle } from "../../../components/errorMessage";
import { MarkdownText } from "../../../components/MarkdownText";
import { canRecoverMessage } from "../../../remote/recovery";
import { colors } from "../../../theme/tokens";
import { Button } from "../../../components/Button";
import { styles } from "./messageStyles";
import type { TimelineCardProps } from "./types";
import { RunIndicator, formatDuration } from "./messageUtils";
import { useCopyState } from "./useCopyState";
import { AttachmentChip } from "./AttachmentChip";
import { StatusDivider } from "./MessageStatus";
import { buildReplyBlocks, SegmentBlock, StepRunBlock } from "./ReplyBlocks";
import { UserMessageText } from "./UserMessageText";

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
  const { t, i18n } = useTranslation();
  const { copied, copy } = useCopyState();

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
      return (
        <View style={styles.assistantMessage}>
          {item.segments && item.segments.length > 0 ? (
            <View style={styles.segmentList}>
              {buildReplyBlocks(item.segments, item.streaming).map(block =>
                block.kind === "steps" ? (
                  <StepRunBlock
                    key={block.segments[0]!.id}
                    segments={block.segments}
                    onResolveToolTarget={onResolveToolTarget}
                  />
                ) : (
                  <SegmentBlock
                    key={block.segment.id}
                    segment={block.segment}
                    streaming={item.streaming && block.segment === item.segments?.at(-1)}
                    onOpenFile={onOpenFile}
                    onResolveToolTarget={onResolveToolTarget}
                  />
                ),
              )}
            </View>
          ) : item.text.trim().length > 0 ? (
            <MarkdownText text={item.text} onOpenFile={onOpenFile} streaming={item.streaming} />
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
              content, above the copy/stats footer. */}
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
              {copyableText.trim() ? (
                <Pressable
                  accessibilityLabel={t("chat.copyResponse")}
                  accessibilityRole="button"
                  hitSlop={8}
                  onPress={() => {
                    Clipboard.setStringAsync(copyableText)
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
    return (
      <View style={styles.userBlock}>
        {item.text.trim().length > 0 && (
          <View style={[styles.message, styles.userMessage]}>
            <UserMessageText onOpenFile={onOpenFile} onOpenSession={onOpenSession} text={item.text} />
          </View>
        )}
        {item.attachments && item.attachments.length > 0 && (
          <View style={styles.attachmentRow}>
            {item.attachments.map(attachment => (
              <AttachmentChip
                attachment={attachment}
                key={`${item.id}:${attachment.path}`}
                onOpen={onOpenAttachment ? () => onOpenAttachment(attachment) : undefined}
              />
            ))}
          </View>
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
