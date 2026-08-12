import {
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronUp,
  CircleAlert,
  Copy,
  FileText,
  Paperclip,
  Pencil,
  TerminalSquare,
  X,
} from "lucide-react-native";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as Clipboard from "expo-clipboard";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { MarkdownText } from "./MarkdownText";
import type { ApprovalPayload, HistoryAttachment, TimelineItem } from "../remote/types";
import { colors, radius, spacing } from "../theme/tokens";
import { Button } from "./Button";

interface TimelineCardProps {
  item: TimelineItem;
  onOpenAttachment?(attachment: HistoryAttachment): void;
}

// Same shape as the desktop footer (desktop/src/lib/date.ts formatDuration): "5s"
// under a minute, "1m 25s" above — so phone and desktop read identically.
function formatDuration(durationMs: number): string {
  const totalSeconds = Math.max(0, Math.round(durationMs / 1_000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  return `${Math.floor(totalSeconds / 60)}m ${totalSeconds % 60}s`;
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

// Tool kinds mirror the desktop activity line (desktop AgentActivityList): the icon
// and label follow the *tool*, not a generic wrench, so phone and desktop read
// the same. The agent's tool_name is exactly one of these four.
type ToolKind = "shell" | "write" | "edit" | "read";

function toolKind(name: string): ToolKind {
  if (name === "shell" || name === "write" || name === "edit" || name === "read") return name;
  return "shell";
}

function ToolGlyph({ kind }: { kind: ToolKind }) {
  const props = { color: colors.inkMuted, size: 14 };
  switch (kind) {
    case "shell":
      return <TerminalSquare {...props} />;
    case "read":
      return <FileText {...props} />;
    case "write":
    case "edit":
      return <Pencil {...props} />;
  }
}

function toolLabel(t: (key: string) => string, kind: ToolKind, complete: boolean): string {
  switch (kind) {
    case "read":
      return t(complete ? "chat.readCompleted" : "chat.reading");
    case "write":
      return t(complete ? "chat.writeCompleted" : "chat.writing");
    case "edit":
      return t(complete ? "chat.editCompleted" : "chat.editing");
    case "shell":
      return t(complete ? "chat.runCompleted" : "chat.runningCommand");
  }
}

// Localized title/summary per approval kind — the agent ships English, so map
// by kind exactly like the desktop ApprovalPrompt, falling back to the wire
// strings for any unknown kind.
const APPROVAL_KIND_I18N: Record<string, { title: string; summary?: string }> = {
  file_read: { title: "approval.readTitle", summary: "approval.readSummary" },
  file_write: { title: "approval.writeTitle", summary: "approval.writeSummary" },
  outside_workspace_write: {
    title: "approval.outsideWriteTitle",
    summary: "approval.outsideWriteSummary",
  },
  shell_command: { title: "approval.shellTitle", summary: "approval.shellSummary" },
  sandbox_escalation: { title: "approval.escalationTitle" },
};

function asRecord(value: unknown): Record<string, unknown> | null {
  if (typeof value === "string") {
    try {
      value = JSON.parse(value);
    } catch {
      return null;
    }
  }
  return value && typeof value === "object" ? (value as Record<string, unknown>) : null;
}

// The file path(s) an approval would touch — surfaced so the user can judge the
// request. Read from the wire `action` (writes[].path / deletes[].path, then
// paths[]); content previews are intentionally omitted on the phone.
function pathEntries(action: Record<string, unknown>, key: string): string[] {
  const entries = Array.isArray(action[key]) ? action[key] : [];
  return entries
    .map(entry => asRecord(entry)?.path as unknown as string)
    .filter((path): path is string => typeof path === "string" && path.length > 0);
}

function approvalPaths(payload: ApprovalPayload): string[] {
  const action = asRecord(payload.action);
  if (!action) return [];
  const fromWrites = pathEntries(action, "writes");
  if (fromWrites.length > 0) return fromWrites;
  const paths = Array.isArray(action.paths) ? action.paths : [];
  return paths.filter((path): path is string => typeof path === "string" && path.length > 0);
}

// Paths an approval would delete (action.deletes) — a delete request is judged
// by what it removes, so deletes render independently from writes/paths.
function approvalDeletes(payload: ApprovalPayload): string[] {
  const action = asRecord(payload.action);
  if (!action) return [];
  return pathEntries(action, "deletes");
}

function approvalCommand(payload: ApprovalPayload): string | null {
  const action = asRecord(payload.action);
  const command = action && typeof action.command === "string" ? action.command : null;
  return command && command.length > 0 ? command : null;
}

interface PendingApprovalCardProps {
  payload: ApprovalPayload;
  submitting: boolean;
  error: string | null;
  onDecision(decision: "approved" | "rejected"): void;
}

// Desktop-styled approval card (neutral surface + warning dot, not the old
// cream/warning fill). Rendered docked above the composer; it vanishes as soon
// as a decision lands because the caller only feeds undecided approvals.
export function PendingApprovalCard({
  payload,
  submitting,
  error,
  onDecision,
}: PendingApprovalCardProps) {
  const { t } = useTranslation();
  const kindI18n = APPROVAL_KIND_I18N[payload.kind ?? ""];
  const titleText = kindI18n
    ? t(kindI18n.title)
    : payload.title || payload.tool_name || t("approval.title");
  const summaryText = kindI18n?.summary ? t(kindI18n.summary) : payload.summary;
  const paths = approvalPaths(payload);
  const deletes = approvalDeletes(payload);
  const command = approvalCommand(payload);
  const isWrite =
    paths.length > 0 &&
    (payload.kind === "file_write" || payload.kind === "outside_workspace_write");
  const detailLabel =
    deletes.length > 0
      ? deletes.length === 1
        ? t("approval.deleteFile")
        : t("approval.deleteFiles", { count: deletes.length })
      : paths.length > 0
        ? isWrite
          ? paths.length === 1
            ? t("approval.writeFile")
            : t("approval.writeFiles", { count: paths.length })
          : t("approval.readPath")
        : command
          ? t("approval.command")
          : null;

  return (
    <View style={styles.approval}>
      <View style={styles.approvalHeader}>
        <View style={styles.approvalDot} />
        <Text numberOfLines={2} style={styles.approvalTitle}>
          {titleText}
        </Text>
      </View>
      {!!summaryText && <Text style={styles.approvalSummary}>{summaryText}</Text>}
      {detailLabel ? (
        <>
          <Text style={styles.approvalDetailLabel}>{detailLabel}</Text>
          <View style={styles.approvalDetail}>
            {command && deletes.length === 0 ? (
              <Text selectable style={styles.approvalPath}>
                {command}
              </Text>
            ) : (
              (deletes.length > 0 ? deletes : paths).map((path, index) => (
                <Text
                  key={`${path}-${index}`}
                  numberOfLines={2}
                  selectable
                  style={styles.approvalPath}
                >
                  {path}
                </Text>
              ))
            )}
          </View>
        </>
      ) : null}
      {!!error && (
        <View style={styles.approvalError}>
          <AlertTriangle color={colors.danger} size={13} />
          <Text style={styles.approvalErrorText}>{error}</Text>
        </View>
      )}
      <View style={styles.approvalActions}>
        <View style={styles.approvalActionLeft}>
          <Button
            compact
            disabled={submitting}
            icon={<X color={colors.ink} size={15} />}
            label={submitting ? t("approval.denying") : t("approval.deny")}
            loading={submitting}
            onPress={() => onDecision("rejected")}
            variant="secondary"
          />
        </View>
        <View style={styles.approvalActionRight}>
          <Button
            compact
            disabled={submitting}
            icon={<Check color={colors.surface} size={15} />}
            label={submitting ? t("approval.allowing") : t("approval.allowOnce")}
            loading={submitting}
            onPress={() => onDecision("approved")}
            variant="primary"
          />
        </View>
      </View>
    </View>
  );
}

function AttachmentChip({
  attachment,
  onOpen,
}: {
  attachment: HistoryAttachment;
  onOpen?(): void;
}) {
  return (
    <Pressable
      accessibilityLabel={attachment.name}
      accessibilityRole="button"
      disabled={!onOpen}
      onPress={onOpen}
      style={({ pressed }) => [styles.attachmentChip, pressed && styles.attachmentChipPressed]}
    >
      {attachment.kind === "file" ? (
        <FileText color={colors.inkSoft} size={13} />
      ) : (
        <Paperclip color={colors.inkSoft} size={13} />
      )}
      <Text numberOfLines={1} style={styles.attachmentName}>
        {attachment.name}
      </Text>
    </Pressable>
  );
}

// Copy feedback parity with the desktop CopyButton: flash a check while the
// clipboard write is in flight/just-done, then settle back to the copy glyph
// after a beat. Only the success path flips the icon — a failed write must
// not masquerade as a copied reply.
function useCopyState(resetMs = 1400) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const copy = () => {
    setCopied(true);
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setCopied(false), resetMs);
  };
  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    [],
  );
  return { copied, copy };
}

export function TimelineCard({ item, onOpenAttachment }: TimelineCardProps) {
  const { t, i18n } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const { copied, copy } = useCopyState();

  if (item.kind === "message") {
    if (item.role === "assistant") {
      // Single "time · N tokens" line, joined like the desktop MessageMeta
      // footer (which renders `parts.join(" · ")`).
      const footerStats = [
        item.durationMs != null ? formatDuration(item.durationMs) : null,
        item.outputTokens != null && item.outputTokens > 0
          ? t("chat.tokens", {
              formattedCount: new Intl.NumberFormat(i18n.language).format(item.outputTokens),
            })
          : null,
      ]
        .filter((part): part is string => !!part)
        .join(" · ");
      return (
        <View style={styles.assistantMessage}>
          {item.text.trim().length > 0 && <MarkdownText text={item.text} />}
          {item.streaming && item.startedAt != null ? (
            // In-flight: the generating indicator occupies the same footer slot
            // the copy button uses once settled (desktop parity), so a streaming
            // reply never shows a copy button.
            <RunIndicator startedAt={item.startedAt} />
          ) : (
            <View style={styles.messageFooter}>
              <Pressable
                accessibilityLabel={t("chat.copyResponse")}
                accessibilityRole="button"
                hitSlop={8}
                onPress={() => {
                  Clipboard.setStringAsync(item.text)
                    .then(() => copy())
                    .catch(() => {});
                }}
                style={styles.copyButton}
              >
                {copied ? (
                  <Check color={colors.accent} size={15} />
                ) : (
                  <Copy color={colors.inkMuted} size={15} />
                )}
              </Pressable>
              {footerStats.length > 0 && <Text style={styles.messageDuration}>{footerStats}</Text>}
            </View>
          )}
        </View>
      );
    }
    return (
      <View style={styles.userBlock}>
        {item.text.trim().length > 0 && (
          <View style={[styles.message, styles.userMessage]}>
            <Text style={[styles.messageText, styles.userText]} selectable>
              {item.text}
            </Text>
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

  if (item.kind === "thinking") {
    return (
      <View style={styles.secondaryCard}>
        <Pressable onPress={() => setExpanded(value => !value)} style={styles.cardHeader}>
          <Text style={styles.cardLabel}>
            {t(item.complete ? "chat.thoughtCompleted" : "chat.thinking")}
          </Text>
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
    const kind = toolKind(item.name);
    // Desktop parity (AgentActivityList): the row carries the call's target —
    // the command for shell, the file path otherwise — but keeps it hidden
    // until tapped; the chevron signals the row is expandable.
    const detail = item.detail?.trim() ? item.detail.trim() : null;
    return (
      <View style={styles.tool}>
        <Pressable
          accessibilityRole="button"
          disabled={!detail}
          onPress={() => setExpanded(value => !value)}
          style={styles.toolHeader}
        >
          <ToolGlyph kind={kind} />
          <Text style={styles.toolText}>{toolLabel(t, kind, item.complete)}</Text>
          {detail ? (
            expanded ? (
              <ChevronUp color={colors.inkMuted} size={15} />
            ) : (
              <ChevronDown color={colors.inkMuted} size={15} />
            )
          ) : null}
        </Pressable>
        {detail && expanded ? (
          <Text selectable style={styles.toolDetailText}>
            {detail}
          </Text>
        ) : null}
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

  // Approval items are rendered docked above the composer (see ChatScreen),
  // never inline in the transcript — so there is nothing to draw here.
  return null;
}

const styles = StyleSheet.create({
  message: {
    maxWidth: "88%",
    borderRadius: radius.lg,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.md,
  },
  userMessage: { alignSelf: "flex-end", backgroundColor: colors.accent },
  userBlock: { alignItems: "flex-end", gap: spacing.xs },
  attachmentRow: {
    maxWidth: "88%",
    flexDirection: "row",
    flexWrap: "wrap",
    justifyContent: "flex-end",
    gap: spacing.xs,
  },
  attachmentChip: {
    maxWidth: 260,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.xs,
    borderWidth: 1,
    borderColor: colors.lineSoft,
    borderRadius: radius.md,
    backgroundColor: colors.surface,
  },
  attachmentChipPressed: { backgroundColor: colors.surfaceSubtle },
  attachmentName: { flexShrink: 1, color: colors.inkSoft, fontSize: 12 },
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
    paddingVertical: spacing.xs,
  },
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
  tool: { paddingHorizontal: spacing.xs, paddingVertical: spacing.xs },
  toolHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  toolText: { color: colors.inkMuted, fontSize: 13, lineHeight: 20 },
  toolDetailText: {
    marginTop: 2,
    paddingLeft: spacing.md + spacing.sm,
    color: colors.inkSoft,
    fontFamily: "monospace",
    fontSize: 12,
    lineHeight: 18,
  },
  notice: { flexDirection: "row", gap: spacing.sm, padding: spacing.md, borderRadius: radius.md },
  warningNotice: { backgroundColor: colors.warningSoft },
  dangerNotice: { backgroundColor: colors.dangerSoft },
  noticeText: { flex: 1, fontSize: 13, lineHeight: 19 },
  approval: {
    padding: spacing.lg,
    gap: spacing.sm,
    borderRadius: radius.lg,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.line,
    shadowColor: colors.inkStrong,
    shadowOpacity: 0.08,
    shadowRadius: 12,
    shadowOffset: { width: 0, height: 4 },
    elevation: 3,
  },
  approvalHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  approvalDot: { width: 8, height: 8, borderRadius: radius.pill, backgroundColor: colors.warning },
  approvalTitle: { flex: 1, color: colors.ink, fontSize: 16, fontWeight: "600", lineHeight: 22 },
  approvalSummary: { color: colors.inkSoft, fontSize: 14, lineHeight: 20 },
  approvalDetailLabel: {
    marginTop: spacing.xs,
    color: colors.inkSoft,
    fontSize: 11,
    fontWeight: "600",
    textTransform: "uppercase",
  },
  approvalDetail: {
    padding: spacing.md,
    borderRadius: radius.md,
    backgroundColor: colors.surfaceSubtle,
    gap: 2,
  },
  approvalPath: { color: colors.ink, fontFamily: "monospace", fontSize: 12, lineHeight: 18 },
  approvalError: { flexDirection: "row", alignItems: "center", gap: spacing.xs },
  approvalErrorText: { flex: 1, color: colors.danger, fontSize: 12, lineHeight: 18 },
  approvalActions: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: spacing.sm,
    marginTop: spacing.sm,
  },
  approvalActionLeft: { flex: 1 },
  approvalActionRight: { flex: 1 },
});
