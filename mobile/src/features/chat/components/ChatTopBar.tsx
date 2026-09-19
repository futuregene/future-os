import { ArrowLeft, FolderOpen } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { colors, layout, radius, spacing } from "../../../theme/tokens";

export function ChatTopBar({
  title,
  contextLabel,
  draft,
  backLabel,
  usageLabel,
  usageText,
  onBack,
  onUsage,
  filesLabel,
  filesOpen,
  onFiles,
}: {
  title: string;
  contextLabel: string;
  draft: boolean;
  backLabel: string;
  /**
   * The conversation's running amount, kept in the bar and tappable for the
   * token/amount breakdown. Renaming lives in that sheet instead of here: the
   * bar keeps one row, and the low-frequency action rides along with the
   * title it edits.
   */
  usageLabel: string;
  usageText: string;
  onBack: () => void;
  onUsage: () => void;
  filesLabel: string;
  filesOpen: boolean;
  onFiles: () => void;
}) {
  return (
    <View style={styles.topbar}>
      <Pressable
        accessibilityLabel={backLabel}
        accessibilityRole="button"
        onPress={onBack}
        style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
      >
        <ArrowLeft color={colors.ink} size={22} />
      </Pressable>
      <View style={styles.titleWrap}>
        <Text numberOfLines={1} style={styles.title}>
          {title}
        </Text>
        <Text numberOfLines={1} style={styles.context} accessibilityLabel={contextLabel}>
          {contextLabel}
        </Text>
      </View>
      {!draft && (
        <Pressable
          accessibilityLabel={filesLabel}
          accessibilityRole="button"
          accessibilityState={{ expanded: filesOpen }}
          onPress={onFiles}
          style={({ pressed }) => [styles.iconButton, (pressed || filesOpen) && styles.pressed]}
        >
          <FolderOpen color={filesOpen ? colors.accent : colors.ink} size={20} />
        </Pressable>
      )}
      {!draft && (
        <Pressable
          accessibilityLabel={usageLabel}
          accessibilityRole="button"
          onPress={onUsage}
          style={({ pressed }) => [styles.usageButton, pressed && styles.pressed]}
        >
          <Text numberOfLines={1} style={styles.usageText}>
            {usageText}
          </Text>
        </Pressable>
      )}
      {draft && <View style={styles.iconButton} />}
    </View>
  );
}

const styles = StyleSheet.create({
  topbar: {
    minHeight: 60,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: spacing.md,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
    backgroundColor: colors.surface,
  },
  iconButton: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  pressed: { backgroundColor: colors.surfaceSubtle },
  titleWrap: { flex: 1, minWidth: 0, alignItems: "center" },
  // Sized to the touch target but allowed to grow with the digits, so the
  // amount never truncates ("¥1,234.5679" stays readable on a narrow phone).
  usageButton: {
    minWidth: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "flex-end",
    justifyContent: "center",
    paddingHorizontal: spacing.sm,
    borderRadius: radius.md,
  },
  usageText: {
    color: colors.inkSoft,
    fontSize: 13,
    fontWeight: "600",
    fontVariant: ["tabular-nums"],
  },
  context: { color: colors.inkSoft, fontSize: 12, maxWidth: "100%", marginTop: 2 },
  title: { color: colors.inkStrong, fontSize: 16, fontWeight: "700", maxWidth: "90%" },
});
