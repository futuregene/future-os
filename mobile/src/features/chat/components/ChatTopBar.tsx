import { ArrowLeft, FolderOpen, ReceiptText } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { colors, layout, radius, spacing } from "../../../theme/tokens";

export function ChatTopBar({
  title,
  contextLabel,
  draft,
  backLabel,
  usageLabel,
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
   * Opens the conversation's token/amount breakdown. An icon rather than the
   * amount: the bar belongs to the conversation, and a number that changes
   * every turn competes with the title for attention. Renaming is not here
   * either — the session list already owns that, so repeating it in the
   * conversation only lengthens the row.
   */
  usageLabel: string;
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
          style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
        >
          <ReceiptText color={colors.ink} size={20} />
        </Pressable>
      )}
      {/* A draft has no session, so neither icon applies; the two spacers keep
          the centered title from jumping sideways when the first message is
          sent and they appear. */}
      {draft && <View style={styles.iconButton} />}
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
  context: { color: colors.inkSoft, fontSize: 12, maxWidth: "100%", marginTop: 2 },
  title: { color: colors.inkStrong, fontSize: 16, fontWeight: "700", maxWidth: "90%" },
});
