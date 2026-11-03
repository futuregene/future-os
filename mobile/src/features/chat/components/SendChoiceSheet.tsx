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
 * rather than a refusal: hold the draft until the run ends, or end the run now
 * and send it (with whatever the queue already holds). The draft is never taken
 * without one of these rows being picked.
 */
export function SendChoiceSheet({
  open,
  queueCount = 0,
  t,
  onClose,
  onInterruptAndSend,
  onQueue,
}: {
  open: boolean;
  /** What is already waiting behind this message, which the interrupt sends too. */
  queueCount?: number;
  t: TFunction;
  onClose: () => void;
  onInterruptAndSend: () => void;
  onQueue: () => void;
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
            onPress={onQueue}
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
                {t(queueCount > 0 ? "chat.queueChoiceInterruptAll" : "chat.queueChoiceInterrupt")}
              </Text>
              <Text style={styles.optionHint}>
                {t(queueCount > 0 ? "chat.queueChoiceInterruptAllHint" : "chat.queueChoiceInterruptHint")}
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
