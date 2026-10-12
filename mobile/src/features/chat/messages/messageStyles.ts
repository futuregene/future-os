import { StyleSheet } from "react-native";
import { chatTypography, colors, layout, radius, spacing } from "../../../theme/tokens";

export const styles = StyleSheet.create({
  message: {
    maxWidth: "92%",
    borderRadius: radius.lg,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
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
  messageText: { color: colors.ink, ...chatTypography },
  userText: { color: colors.surface },
  userMention: { color: colors.surface, fontWeight: "600" },
  userLink: { color: colors.surface, fontWeight: "600", textDecorationLine: "underline" },
  terminationNotice: {
    marginTop: spacing.md,
  },
  terminationNoticeText: {
    color: colors.inkSoft,
    fontSize: 14,
    lineHeight: 24,
    marginTop: spacing.sm,
  },
  stoppedNoticeText: { color: colors.inkMuted },
  // Always-visible actions and stats share the reply's left edge with its timer.
  messageFooter: {
    alignSelf: "flex-start",
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "flex-start",
    gap: spacing.md,
    marginTop: spacing.sm,
  },
  segmentList: { gap: spacing.sm, marginTop: spacing.xs },
  // Add space after top-level activity blocks, without changing expanded rows.
  activityBlock: { marginBottom: spacing.xs },
  inlineThinking: {
    paddingVertical: 2,
    gap: 2,
  },
  // Shrink-wrap the collapsed header while keeping its leading edge aligned.
  railThinking: {
    alignSelf: "flex-start",
    borderRightWidth: 2,
    borderRightColor: colors.line,
    paddingRight: spacing.md,
  },
  inlineThinkingHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
  },
  inlineThinkingLabel: { color: colors.inkMuted, fontSize: 13, lineHeight: 20 },
  inlineThinkingText: {
    borderLeftWidth: 2,
    borderLeftColor: colors.line,
    paddingLeft: spacing.md,
    color: colors.inkSoft,
    fontSize: 13,
    lineHeight: 19,
    marginTop: 2,
  },
  inlineTool: {
    paddingVertical: 2,
    gap: 2,
  },
  // Collapsed badges stay compact and start at the reply's left edge.
  railBlock: { alignSelf: "flex-start" },
  railRow: { justifyContent: "flex-start" },
  // Opened rows are reading content: left-aligned, and indented like the tool
  // burst children they belong to.
  inlineToolChildren: {
    gap: 2,
    marginTop: 2,
    paddingLeft: spacing.md + spacing.sm,
  },
  // A burst child that can still fetch its command sits its glyph beside the
  // line, so the row reads as tappable without moving the text off the other
  // children's left edge.
  inlineToolChildRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
    alignSelf: "flex-start",
  },
  // Keep the left-aligned counts in one compact cluster.
  stepSummaryRow: {
    alignSelf: "flex-start",
    justifyContent: "flex-start",
    gap: spacing.xs,
  },
  // The icon and its count are one token: no gap between them, so the wrench and
  // its `×5` read as a unit. The row's gap spaces the kinds around the `·`.
  stepSummaryItem: {
    flexDirection: "row",
    alignItems: "center",
    gap: 0,
  },
  // The expanded rows fall back into the reading column, indented one step so
  // they read as belonging to the summary line above them.
  stepChildren: {
    gap: spacing.xs,
    marginTop: 2,
    paddingLeft: spacing.md + spacing.sm,
  },
  inlineToolChild: {
    flexShrink: 1,
    color: colors.inkSoft,
    fontFamily: "monospace",
    fontSize: 12,
    lineHeight: 18,
  },
  compactionDivider: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingVertical: 4,
  },
  compactionLine: { flex: 1, height: StyleSheet.hairlineWidth, backgroundColor: colors.line },
  compactionLabel: { color: colors.inkMuted, fontSize: 12 },
  messageDuration: { color: colors.inkMuted, fontSize: 12 },
  recoveryRow: {
    flexDirection: "row",
    gap: spacing.sm,
    marginTop: spacing.sm,
  },
  copyButton: {
    minWidth: layout.touchTarget,
    minHeight: layout.touchTarget,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-start",
    paddingVertical: spacing.xs,
  },
  // Keep the live timer aligned with the settled footer.
  runIndicator: {
    flexDirection: "row",
    alignItems: "center",
    alignSelf: "flex-start",
    gap: spacing.sm,
    marginTop: spacing.sm,
  },
  runDot: { width: 14, height: 14, borderRadius: radius.pill, backgroundColor: colors.generating },
  runDuration: { color: colors.inkMuted, fontSize: 12 },
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
  // The header is one line again now that the target dropped below it, so its
  // glyph, label and chevron centre against each other instead of top-aligning.
  toolHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  toolText: { color: colors.inkMuted, fontSize: 13, lineHeight: 20 },
  // The target under a tool row — a file name or a shell command — indents to
  // the label it belongs to and wraps at full width: a phone cannot truncate a
  // name into uselessness, and the row is no longer shrink-wrapped so there is
  // width for a second line.
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
  approvalCommandToggle: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
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
