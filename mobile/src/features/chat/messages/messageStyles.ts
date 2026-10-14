import { StyleSheet } from "react-native";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles } from "../../../theme/ThemeContext";

export const useStyles = createThemedStyles(theme => StyleSheet.create({
  message: {
    maxWidth: "92%",
    borderRadius: radius.lg,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
  },
  userMessage: { alignSelf: "flex-end", backgroundColor: theme.colors.accent },
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
    borderColor: theme.colors.lineSoft,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surface,
  },
  attachmentChipPressed: { backgroundColor: theme.colors.surfaceSubtle },
  attachmentName: { flexShrink: 1, color: theme.colors.inkSoft, fontSize: theme.font(12) },
  assistantMessage: { alignSelf: "stretch", paddingHorizontal: spacing.xs },
  messageText: { color: theme.colors.ink, ...theme.chat },
  userText: { color: theme.colors.surface },
  userMention: { color: theme.colors.surface, fontWeight: "600" },
  userLink: { color: theme.colors.surface, fontWeight: "600", textDecorationLine: "underline" },
  terminationNotice: {
    marginTop: spacing.md,
  },
  terminationNoticeText: {
    color: theme.colors.inkSoft,
    fontSize: theme.font(14),
    lineHeight: theme.leading(24),
    marginTop: spacing.sm,
  },
  stoppedNoticeText: { color: theme.colors.inkMuted },
  // Always-visible actions and stats share the reply's left edge with its timer.
  messageFooter: {
    alignSelf: "flex-start",
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    justifyContent: "flex-start",
    gap: spacing.sm,
    marginTop: spacing.sm,
  },
  segmentList: { gap: spacing.sm, marginTop: spacing.xs },
  // Add space after top-level activity blocks, without changing expanded rows.
  activityBlock: { marginBottom: spacing.xs },
  inlineThinking: {
    paddingVertical: 2,
    gap: 2,
  },
  inlineThinkingHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
  },
  inlineThinkingLabel: { color: theme.colors.inkMuted, fontSize: theme.font(13), lineHeight: theme.leading(20) },
  // The revealed reasoning sits under its label exactly like a tool row's
  // revealed target: one step in, and with no rule of its own.
  inlineThinkingText: {
    paddingLeft: spacing.md + spacing.sm,
    color: theme.colors.inkSoft,
    fontSize: theme.font(13),
    lineHeight: theme.leading(19),
    marginTop: 2,
  },
  inlineTool: {
    paddingVertical: 2,
    gap: 2,
  },
  // A collapsed row — a tool call or a reasoning slice — stays compact and
  // starts at the reply's left edge: glyph, words, disclosure, nothing else.
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
    color: theme.colors.inkSoft,
    fontFamily: "monospace",
    fontSize: theme.font(12),
    lineHeight: theme.leading(18),
  },
  compactionDivider: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
    paddingVertical: 4,
  },
  compactionLine: { flex: 1, height: StyleSheet.hairlineWidth, backgroundColor: theme.colors.line },
  compactionLabel: { color: theme.colors.inkMuted, fontSize: theme.font(12) },
  messageDuration: { color: theme.colors.inkMuted, fontSize: theme.font(12) },
  recoveryRow: {
    flexDirection: "row",
    gap: spacing.sm,
    marginTop: spacing.sm,
  },
  // The copy/fork glyphs are 15pt. Left-aligned in a full 44pt box they sat 43pt
  // apart — wider than the gap to the stats text — so the two actions stopped
  // reading as one group. Hug the glyph instead and let the 8pt `hitSlop` on each
  // pressable keep the 44pt (`layout.touchTarget`) tap area.
  copyButton: {
    minWidth: 28,
    minHeight: layout.touchTarget,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
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
  runDot: { width: 14, height: 14, borderRadius: radius.pill, backgroundColor: theme.colors.generating },
  runDuration: { color: theme.colors.inkMuted, fontSize: theme.font(12) },
  secondaryCard: {
    borderLeftWidth: 2,
    borderLeftColor: theme.colors.line,
    paddingLeft: spacing.md,
    paddingVertical: spacing.xs,
  },
  cardHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  cardLabel: { color: theme.colors.inkSoft, fontSize: theme.font(13), fontWeight: "600" },
  secondaryText: { color: theme.colors.inkSoft, fontSize: theme.font(13), lineHeight: theme.leading(19), marginTop: spacing.sm },
  tool: { paddingHorizontal: spacing.xs, paddingVertical: spacing.xs },
  // The header is one line again now that the target dropped below it, so its
  // glyph, label and chevron centre against each other instead of top-aligning.
  toolHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  toolText: { color: theme.colors.inkMuted, fontSize: theme.font(13), lineHeight: theme.leading(20) },
  // The target under a tool row — a file name or a shell command — indents to
  // the label it belongs to and wraps at full width: a phone cannot truncate a
  // name into uselessness, and the row is no longer shrink-wrapped so there is
  // width for a second line.
  toolDetailText: {
    marginTop: 2,
    paddingLeft: spacing.md + spacing.sm,
    color: theme.colors.inkSoft,
    fontFamily: "monospace",
    fontSize: theme.font(12),
    lineHeight: theme.leading(18),
  },
  notice: { flexDirection: "row", gap: spacing.sm, padding: spacing.md, borderRadius: radius.md },
  warningNotice: { backgroundColor: theme.colors.warningSoft },
  dangerNotice: { backgroundColor: theme.colors.dangerSoft },
  noticeText: { flex: 1, fontSize: theme.font(13), lineHeight: theme.leading(19) },
  approval: {
    padding: spacing.lg,
    gap: spacing.sm,
    borderRadius: radius.lg,
    backgroundColor: theme.colors.surface,
    borderWidth: 1,
    borderColor: theme.colors.line,
    shadowColor: theme.colors.inkStrong,
    shadowOpacity: 0.08,
    shadowRadius: 12,
    shadowOffset: { width: 0, height: 4 },
    elevation: 3,
  },
  approvalHeader: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
  approvalDot: { width: 8, height: 8, borderRadius: radius.pill, backgroundColor: theme.colors.warning },
  approvalTitle: { flex: 1, color: theme.colors.ink, fontSize: theme.font(16), fontWeight: "600", lineHeight: theme.leading(22) },
  approvalSummary: { color: theme.colors.inkSoft, fontSize: theme.font(14), lineHeight: theme.leading(20) },
  approvalDetailLabel: {
    marginTop: spacing.xs,
    color: theme.colors.inkSoft,
    fontSize: theme.font(11),
    fontWeight: "600",
    textTransform: "uppercase",
  },
  approvalDetail: {
    padding: spacing.md,
    borderRadius: radius.md,
    backgroundColor: theme.colors.surfaceSubtle,
    gap: 2,
  },
  approvalCommandToggle: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  approvalPath: { color: theme.colors.ink, fontFamily: "monospace", fontSize: theme.font(12), lineHeight: theme.leading(18) },
  approvalError: { flexDirection: "row", alignItems: "center", gap: spacing.xs },
  approvalErrorText: { flex: 1, color: theme.colors.danger, fontSize: theme.font(12), lineHeight: theme.leading(18) },
  approvalActions: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: spacing.sm,
    marginTop: spacing.sm,
  },
  approvalActionLeft: { flex: 1 },
  approvalActionRight: { flex: 1 },
}));
