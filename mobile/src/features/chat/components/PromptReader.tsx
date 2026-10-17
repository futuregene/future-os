import { useTranslation } from "react-i18next";
import { Check, Copy, X } from "lucide-react-native";
import * as Clipboard from "expo-clipboard";
import { Modal, Pressable, StyleSheet, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { MarkdownText } from "../../../components/MarkdownText";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";
import { useCopyState } from "../messages/useCopyState";
import { previewSurfaceInsets } from "./previewInsets";

/**
 * The whole of a long prompt, on a surface of its own.
 *
 * A prompt taller than a few lines cannot be drawn inside the transcript: the
 * reporting device painted such a bubble's fill and none of its text, and the
 * failure is a function of the row's height rather than of the words (see
 * `promptCollapse`). So the bubble keeps a preview and this reader shows the
 * rest — the same way a long document is read from its preview rather than from
 * a chat bubble.
 *
 * The body is the file preview's own markdown mode, which paints one list row
 * per block: no single view here is taller than a paragraph, so the shape that
 * broke the transcript cannot occur. The prompt is drawn on the app's surface
 * (default tone), not on the accent fill the bubble uses, because this is a
 * document to read rather than the reader's own bubble.
 */
export function PromptReader({
  text,
  visible,
  onClose,
}: {
  text: string;
  visible: boolean;
  onClose: () => void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const insets = useSafeAreaInsets();
  const { copied, copy } = useCopyState();
  return (
    <Modal
      animationType="slide"
      onRequestClose={onClose}
      presentationStyle="pageSheet"
      // Same reason as the file reader: draw under the status bar deliberately
      // and inset the surface back out of it, so the header's buttons are not
      // under the system bar's hit area.
      statusBarTranslucent
      visible={visible}
    >
      <View style={[styles.surface, previewSurfaceInsets(insets)]}>
        <View style={styles.header}>
          <Text accessibilityRole="header" numberOfLines={1} style={styles.title}>
            {t("chat.promptTitle")}
          </Text>
          <Pressable
            accessibilityLabel={t("chat.copy")}
            accessibilityRole="button"
            onPress={() => {
              Clipboard.setStringAsync(text).then(copy).catch(() => { });
            }}
            style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
          >
            {copied
              ? <Check color={colors.accent} size={18} />
              : <Copy color={colors.ink} size={18} />}
          </Pressable>
          <Pressable
            accessibilityLabel={t("common.close")}
            accessibilityRole="button"
            onPress={onClose}
            style={({ pressed }) => [styles.iconButton, pressed && styles.pressed]}
          >
            <X color={colors.ink} size={22} />
          </Pressable>
        </View>
        <MarkdownText mode="file-preview" text={text} />
      </View>
    </Modal>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  surface: { flex: 1, backgroundColor: theme.colors.surface },
  header: {
    minHeight: 60,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.sm,
    borderBottomWidth: 1,
    borderBottomColor: theme.colors.lineSoft,
  },
  title: { flex: 1, minWidth: 0, color: theme.colors.inkStrong, fontSize: theme.font(16), fontWeight: "700" },
  iconButton: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
}));
