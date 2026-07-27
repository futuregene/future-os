import { Check, ChevronDown, ChevronUp, CircleAlert, Copy, Wrench, X } from "lucide-react-native";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Clipboard, Pressable, StyleSheet, Text, View } from "react-native";
import { MarkdownText } from "./MarkdownText";
import type { TimelineItem } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";
import { Button } from "./Button";

interface TimelineCardProps {
  item: TimelineItem;
  onDecision(id: string, decision: "approved" | "rejected"): Promise<void>;
}

function formatDuration(durationMs: number): string {
  return `${Math.max(1, Math.round(durationMs / 1_000))}s`;
}

function RunIndicator({ startedAt }: { startedAt: number }) {
  const [now, setNow] = useState(startedAt);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(timer);
  }, []);
  return (
    <View style={styles.runIndicator}>
      <View style={styles.runDot} />
      <Text style={styles.runDuration}>{formatDuration(now - startedAt)}</Text>
    </View>
  );
}

export function TimelineCard({ item, onDecision }: TimelineCardProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  if (item.kind === "message") {
    if (item.role === "assistant") {
      return (
        <View style={styles.assistantMessage}>
          <MarkdownText text={item.text} />
          {item.durationMs != null && (
            <View style={styles.messageFooter}>
              <Text style={styles.messageDuration}>{formatDuration(item.durationMs)}</Text>
              <Pressable
                accessibilityLabel="Copy response"
                accessibilityRole="button"
                hitSlop={8}
                onPress={() => Clipboard.setString(item.text)}
                style={styles.copyButton}
              >
                <Copy color={colors.inkMuted} size={16} />
                <Text style={styles.copyLabel}>Copy</Text>
              </Pressable>
            </View>
          )}
        </View>
      );
    }
    return (
      <View style={[styles.message, styles.userMessage]}>
        <Text style={[styles.messageText, styles.userText]} selectable>
          {item.text}
        </Text>
      </View>
    );
  }

  if (item.kind === "run") return <RunIndicator startedAt={item.startedAt} />;

  if (item.kind === "thinking") {
    return (
      <View style={styles.secondaryCard}>
        <Pressable onPress={() => setExpanded(value => !value)} style={styles.cardHeader}>
          <Text style={styles.cardLabel}>{t("chat.thinking")}</Text>
          {expanded ? (
            <ChevronUp color={colors.inkMuted} size={17} />
          ) : (
            <ChevronDown color={colors.inkMuted} size={17} />
          )}
        </Pressable>
        {expanded && <Text style={styles.secondaryText}>{item.text}</Text>}
      </View>
    );
  }

  if (item.kind === "tool") {
    return (
      <View style={styles.tool}>
        <Wrench color={item.complete ? colors.success : colors.warning} size={16} />
        <Text style={styles.toolText}>
          {t(item.complete ? "chat.toolDone" : "chat.toolRunning", { name: item.name })}
        </Text>
      </View>
    );
  }

  if (item.kind === "notice") {
    const warning = item.tone === "warning";
    return (
      <View style={[styles.notice, warning ? styles.warningNotice : styles.dangerNotice]}>
        <CircleAlert color={warning ? colors.warning : colors.danger} size={17} />
        <Text style={[styles.noticeText, { color: warning ? colors.warning : colors.danger }]}>
          {item.text === "truncated" ? t("chat.truncated") : item.text}
        </Text>
      </View>
    );
  }

  const payload = item.payload;
  const submit = async (decision: "approved" | "rejected") => {
    setSubmitting(true);
    try {
      await onDecision(payload.approval_request_id, decision);
    } finally {
      setSubmitting(false);
    }
  };
  return (
    <View style={styles.approval}>
      <Text style={styles.approvalKicker}>{t("approval.title")}</Text>
      <Text style={styles.approvalTitle}>
        {payload.title || payload.tool_name || t("approval.title")}
      </Text>
      {!!payload.summary && <Text style={styles.approvalSummary}>{payload.summary}</Text>}
      {item.decision ? (
        <View style={styles.decision}>
          {item.decision === "approved" ? (
            <Check color={colors.success} size={17} />
          ) : (
            <X color={colors.danger} size={17} />
          )}
          <Text style={item.decision === "approved" ? styles.approved : styles.rejected}>
            {t(item.decision === "approved" ? "approval.approved" : "approval.rejected")}
          </Text>
        </View>
      ) : (
        <View style={styles.actions}>
          <View style={styles.action}>
            <Button
              compact
              disabled={submitting}
              label={t("approval.reject")}
              onPress={() => void submit("rejected")}
              variant="danger"
            />
          </View>
          <View style={styles.action}>
            <Button
              compact
              disabled={submitting}
              label={t("approval.approve")}
              onPress={() => void submit("approved")}
            />
          </View>
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  message: {
    maxWidth: "88%",
    borderRadius: radius.lg,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.md,
  },
  userMessage: { alignSelf: "flex-end", backgroundColor: colors.accent },
  assistantMessage: { alignSelf: "stretch", paddingHorizontal: spacing.xs },
  messageText: { color: colors.ink, fontSize: 15, lineHeight: 22 },
  userText: { color: colors.surface },
  messageFooter: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    marginTop: -spacing.xs,
  },
  messageDuration: { color: colors.inkMuted, fontSize: 12 },
  copyButton: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingVertical: spacing.xs,
  },
  copyLabel: { color: colors.inkMuted, fontSize: 12, fontWeight: "600" },
  runIndicator: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.xs,
    paddingVertical: spacing.sm,
  },
  runDot: { width: 14, height: 14, borderRadius: radius.pill, backgroundColor: colors.generating },
  runDuration: { color: colors.inkMuted, fontSize: 16 },
  secondaryCard: {
    borderLeftWidth: 2,
    borderLeftColor: colors.line,
    paddingLeft: spacing.md,
    paddingVertical: spacing.xs,
  },
  cardHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  cardLabel: { color: colors.inkSoft, fontSize: 13, fontWeight: "600" },
  secondaryText: { color: colors.inkSoft, fontSize: 13, lineHeight: 19, marginTop: spacing.sm },
  tool: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    padding: spacing.md,
    borderRadius: radius.md,
    backgroundColor: colors.surfaceSubtle,
  },
  toolText: { flex: 1, color: colors.inkSoft, fontSize: 13, fontWeight: "500" },
  notice: { flexDirection: "row", gap: spacing.sm, padding: spacing.md, borderRadius: radius.md },
  warningNotice: { backgroundColor: colors.warningSoft },
  dangerNotice: { backgroundColor: colors.dangerSoft },
  noticeText: { flex: 1, fontSize: 13, lineHeight: 19 },
  approval: {
    padding: spacing.lg,
    gap: spacing.sm,
    borderRadius: radius.md,
    backgroundColor: colors.warningSoft,
    borderWidth: 1,
    borderColor: colors.warningLine,
  },
  approvalKicker: {
    color: colors.warning,
    fontSize: 12,
    fontWeight: "700",
    textTransform: "uppercase",
  },
  approvalTitle: { color: colors.inkStrong, fontSize: 16, fontWeight: "700" },
  approvalSummary: { color: colors.inkSoft, fontSize: 14, lineHeight: 20 },
  actions: { flexDirection: "row", gap: spacing.sm, marginTop: spacing.sm },
  action: { flex: 1 },
  decision: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  approved: { color: colors.success, fontWeight: "600" },
  rejected: { color: colors.danger, fontWeight: "600" },
});
