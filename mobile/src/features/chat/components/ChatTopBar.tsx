import { ArrowLeft, FolderOpen, Pencil } from "lucide-react-native";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { colors, layout, radius, spacing } from "../../../theme/tokens";

export function ChatTopBar({
  title,
  draft,
  backLabel,
  renameLabel,
  onBack,
  onRename,
  filesLabel,
  filesOpen,
  onFiles,
}: {
  title: string;
  draft: boolean;
  backLabel: string;
  renameLabel: string;
  onBack: () => void;
  onRename: () => void;
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
          accessibilityLabel={renameLabel}
          accessibilityRole="button"
          onPress={onRename}
          style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
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
  title: { color: colors.inkStrong, fontSize: 16, fontWeight: "700", maxWidth: "90%" },
});
