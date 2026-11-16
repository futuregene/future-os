import { Pencil, X } from "lucide-react-native";
import { useEffect } from "react";
import { Keyboard, Modal, Pressable, StyleSheet, Text, View } from "react-native";
import type { TFunction } from "i18next";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { ChoiceOptionRow } from "../../../components/ChoiceCard";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";
import type { ChoiceAnswer } from "../choiceAnswer";

/**
 * Changing the answer to a `futureos-choice` card, on a phone.
 *
 * 390pt has no room for the desktop's in-place popover, so the pencil above the
 * field opens this bottom sheet instead (design
 * `docs/design/choice-cards.zh-CN.md` §5.7): the same options, the same 44pt
 * rows, and a pick written straight back to the chip — the draft's text is never
 * touched.
 */
export function ChoiceSheet({
  answer,
  onClose,
  onPick,
  open,
  t,
}: {
  answer: ChoiceAnswer | null;
  onClose: () => void;
  onPick: (optionId: string) => void;
  open: boolean;
  t: TFunction;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  // Opened from a dock whose keyboard is up; a RN Modal is its own window, so a
  // bottom-anchored sheet would sit behind it. Dismiss the keyboard first, as the
  // model picker and the send sheet do.
  useEffect(() => {
    if (open)
      Keyboard.dismiss();
  }, [open]);
  return (
    <Modal
      animationType="slide"
      onRequestClose={onClose}
      transparent
      visible={open && answer !== null}
    >
      <View style={[styles.overlay, {
        paddingBottom: insets.bottom + layout.gutter,
        paddingLeft: insets.left + layout.gutter,
        paddingRight: insets.right + layout.gutter,
      }]}
      >
        <Pressable accessible={false} onPress={onClose} style={StyleSheet.absoluteFill} />
        {answer ? (
          <View accessibilityViewIsModal style={styles.sheet} testID="choice-sheet">
            <View style={styles.header}>
              <Pencil color={colors.inkSoft} size={14} />
              <Text accessibilityRole="header" style={styles.title}>
                {t("choice.sheet.title", { title: answer.title })}
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
            <View style={styles.options}>
              {answer.options.map(option => (
                <ChoiceOptionRow
                  key={option.id}
                  mode={answer.mode}
                  onPick={() => onPick(option.id)}
                  option={option}
                  picked={answer.selectedIds.includes(option.id)}
                  testID={`choice-sheet-option-${option.id}`}
                />
              ))}
            </View>
            <Text style={styles.hint}>{t("choice.sheet.hint")}</Text>
          </View>
        ) : null}
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
    paddingTop: 14,
    paddingHorizontal: 6,
    paddingBottom: spacing.lg,
    borderRadius: radius.lg,
    borderBottomLeftRadius: radius.xl,
    borderBottomRightRadius: radius.xl,
    backgroundColor: theme.colors.surface,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingLeft: spacing.sm,
    paddingBottom: spacing.sm,
  },
  title: {
    flex: 1,
    minWidth: 0,
    color: theme.colors.ink,
    fontSize: theme.font(14),
    fontWeight: "600",
  },
  close: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.pill,
  },
  options: { gap: 2 },
  hint: {
    paddingTop: 10,
    paddingHorizontal: spacing.sm,
    color: theme.colors.inkSoft,
    fontSize: theme.font(12),
    lineHeight: theme.leading(17),
  },
  pressed: { opacity: 0.72 },
}));
