import { ArrowLeft, Pencil } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { colors, radius, spacing } from "../../../theme/tokens";

export function ChatTopBar({
  title,
  draft,
  backLabel,
  renameLabel,
  onBack,
  onRename,
}: {
  title: string;
  draft: boolean;
  backLabel: string;
  renameLabel: string;
  onBack: () => void;
  onRename: () => void;
}) {
  return (
    <View style={styles.topbar}>
      <Pressable
        accessibilityLabel={backLabel}
        accessibilityRole="button"
        onPress={onBack}
        style={styles.iconButton}
      >
        <ArrowLeft color={colors.ink} size={22} />
      </Pressable>
      <View style={styles.titleWrap}>
        <Text numberOfLines={1} style={styles.title}>
          {title}
        </Text>
      </View>
      {!draft && (
        <Pressable
          accessibilityLabel={renameLabel}
          accessibilityRole="button"
          onPress={onRename}
          style={styles.iconButton}
        >
          <Pencil color={colors.ink} size={18} />
        </Pressable>
      )}
      {draft && <View style={styles.iconButton} />}
    </View>
  );
}

const styles = StyleSheet.create({
  topbar: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: spacing.md,
    borderBottomWidth: 1,
    borderBottomColor: colors.lineSoft,
    backgroundColor: colors.surface,
  },
  iconButton: {
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  titleWrap: { flex: 1, alignItems: "center" },
  title: { color: colors.inkStrong, fontSize: 16, fontWeight: "700", maxWidth: "90%" },
});
