import { ListPlus, Zap, X } from "lucide-react-native";
import { useEffect } from "react";
import { Keyboard, Modal, Pressable, StyleSheet, Text, View } from "react-native";
import type { TFunction } from "i18next";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";

/**
 * What "send" means while this conversation is still answering.
 *
 * The desktop answers one prompt at a time, so a send during a run is a decision
 * rather than a refusal: hand it to the session (which holds it and runs it when
 * the current turn ends), or hand it over and end the run it waited behind. The
 * draft is never taken without one of these rows being picked.
 *
 * The first row used to fill a queue this phone kept in memory; it now means the
 * same thing it means on the desktop: the message is the session's, at once.
 */
export function SendChoiceSheet({
  open,
  t,
  onClose,
  onInterruptAndSend,
  onSend,
}: {
  open: boolean;
  t: TFunction;
  onClose: () => void;
  onInterruptAndSend: () => void;
  /** Hand the message to the session, which runs it at the turn boundary. */
  onSend: () => void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  // The sheet is opened from the toolbar while the keyboard is up; a RN Modal is
  // its own window, so a bottom-anchored card would sit behind it. Dismiss the
  // keyboard so both rows are reachable, as the model picker does.
  useEffect(() => {
    if (open)
      Keyboard.dismiss();
  }, [open]);
  return (
    <Modal animationType="slide" onRequestClose={onClose} transparent visible={open}>
      <View style={[styles.overlay, {
        paddingBottom: insets.bottom + layout.gutter,
        paddingLeft: insets.left + layout.gutter,
        paddingRight: insets.right + layout.gutter,
      }]}
      >
        <Pressable accessible={false} onPress={onClose} style={StyleSheet.absoluteFill} />
        <View accessibilityViewIsModal style={styles.sheet}>
          <View style={styles.header}>
            <Text accessibilityRole="header" style={styles.title}>
              {t("chat.chooseSendTitle")}
            </Text>
            <Pressable
              accessibilityLabel={t("common.close")}
              accessibilityRole="button"
              onPress={onClose}
              style={({ pressed }) => [styles.close, pressed && styles.pressed]}
            >
              <X color={colors.inkSoft} size={20} />
            </Pressable>
          </View>
          <Pressable
            accessibilityRole="button"
            onPress={onSend}
            style={({ pressed }) => [styles.option, pressed && styles.pressed]}
            testID="send-choice-queue"
          >
            <ListPlus color={colors.accent} size={20} />
            <View style={styles.optionCopy}>
              <Text style={styles.optionLabel}>{t("chat.queueChoiceEnqueue")}</Text>
              <Text style={styles.optionHint}>{t("chat.queueChoiceEnqueueHint")}</Text>
            </View>
          </Pressable>
          <Pressable
            accessibilityRole="button"
            onPress={onInterruptAndSend}
            style={({ pressed }) => [styles.option, pressed && styles.pressed]}
            testID="send-choice-interrupt"
          >
            <Zap color={colors.warning} size={20} />
            <View style={styles.optionCopy}>
              <Text style={styles.optionLabel}>
                {t("chat.queueChoiceInterrupt")}
              </Text>
              <Text style={styles.optionHint}>
                {t("chat.queueChoiceInterruptHint")}
              </Text>
            </View>
          </Pressable>
        </View>
      </View>
    </Modal>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  overlay: {
    flex: 1,
    justifyContent: "flex-end",
    backgroundColor: theme.colors.overlay,
  },
  sheet: {
    width: "100%",
    maxWidth: layout.formMaxWidth,
    alignSelf: "center",
    padding: spacing.sm,
    borderRadius: radius.xl,
    backgroundColor: theme.colors.surface,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    paddingLeft: spacing.md,
    marginBottom: spacing.xs,
  },
  title: {
    flex: 1,
    paddingVertical: spacing.sm,
    color: theme.colors.inkStrong,
    fontSize: theme.font(16),
    fontWeight: "700",
  },
  close: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.pill,
  },
  option: {
    minHeight: 64,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.md,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderRadius: radius.md,
  },
  optionCopy: { minWidth: 0, flex: 1 },
  optionLabel: {
    color: theme.colors.ink,
    fontSize: theme.font(15),
    fontWeight: "600",
  },
  optionHint: {
    marginTop: 2,
    color: theme.colors.inkMuted,
    fontSize: theme.font(12),
  },
  pressed: { opacity: 0.72 },
}));
