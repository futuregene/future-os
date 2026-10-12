import { StyleSheet } from "react-native";

import { colors, layout, radius, spacing } from "../../theme/tokens";

export const styles = StyleSheet.create({
  safe: { flex: 1, backgroundColor: colors.surface },
  keyboard: {
    flex: 1,
    width: "100%",
    maxWidth: layout.contentMaxWidth,
    alignSelf: "center",
    backgroundColor: colors.surface,
  },
  chatContent: { flex: 1, minHeight: 0 },
  timelineList: { flex: 1, minHeight: 0 },
  // Overlay, not a list row: changing sync state must not shift the viewport.
  syncNotice: {
    position: "absolute",
    top: spacing.xs,
    left: layout.gutter,
    right: layout.gutter,
    zIndex: 2,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: spacing.sm,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    backgroundColor: colors.surfaceSubtle,
    borderRadius: radius.md,
  },
  syncNoticeText: { flexShrink: 1, color: colors.inkMuted, fontSize: 12 },
  // `inverted` flips the visual axis. A flexible physical header consumes only
  // the unused height of an underfilled list and therefore becomes visual
  // bottom space, leaving short conversations at the visual top. It collapses
  // to zero once message rows overflow the viewport.
  timeline: {
    flexGrow: 1,
    paddingHorizontal: layout.gutter,
    paddingVertical: spacing.sm,
  },
  timelineFlexSpacer: { flexGrow: 1 },
  emptyTimeline: {
    flexGrow: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  empty: { color: colors.inkMuted, fontSize: 14 },
  loadingState: {
    alignItems: "center",
    gap: spacing.sm,
    paddingVertical: spacing.xl,
  },
  historyError: { color: colors.inkMuted, fontSize: 14, textAlign: "center" },
  retryButton: {
    minHeight: layout.touchTarget,
    justifyContent: "center",
    backgroundColor: colors.accent,
    borderRadius: radius.pill,
    paddingHorizontal: spacing.lg,
    paddingVertical: spacing.sm,
  },
  retryPressed: { opacity: 0.75 },
  retryLabel: { color: colors.surface, fontSize: 14, fontWeight: "600" },
  itemGap: { height: spacing.md },
  timelineViewport: { flex: 1 },
  // The question control sits above the composer's edge, clear of the message
  // column and of the load-older/sync notices that own the viewport's top.
  questionNav: { right: layout.gutter, bottom: spacing.lg },
  loadOlder: { top: spacing.sm },
  syncNoticeBelowHistory: { top: spacing.sm + layout.touchTarget + spacing.sm },
  transferTrack: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 2,
    overflow: "hidden",
  },
  transferFill: {
    height: 2,
    borderRadius: radius.pill,
    backgroundColor: colors.accent,
  },
});
