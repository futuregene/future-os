import { ArrowDown, ArrowUp } from "lucide-react-native";
import {
  Pressable,
  StyleSheet,
  View,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { colors, layout, spacing } from "../../../theme/tokens";

/**
 * "↑ previous question / ↓ next question" for a long transcript.
 *
 * Two round buttons stacked on the right edge (the chosen of four options on the
 * style sheet): at 44px each they are the platform's minimum touch target, and
 * stacking keeps them clear of the reading column instead of covering a line of
 * it. ↑ is offered without having scrolled first — a conversation opens at its
 * tail, and going back one turn is the first thing a reader of a long answer
 * wants — while ↓ waits until the tail is off screen, since at the tail there is
 * nothing newer below. A direction with nothing to go to stays in place and
 * inert, so the pair does not move under the thumb as the reader scrolls.
 */
export function QuestionNavControl({
  previousLabel,
  nextLabel,
  hasPrevious,
  hasNext,
  onPrevious,
  onNext,
  style,
}: {
  previousLabel: string;
  nextLabel: string;
  hasPrevious: boolean;
  hasNext: boolean;
  onPrevious: () => void;
  onNext: () => void;
  style: StyleProp<ViewStyle>;
}) {
  return (
    <View style={[styles.stack, style]}>
      <ArrowButton
        disabled={!hasPrevious}
        label={previousLabel}
        onPress={onPrevious}
        icon="up"
      />
      <ArrowButton
        disabled={!hasNext}
        label={nextLabel}
        onPress={onNext}
        icon="down"
      />
    </View>
  );
}

function ArrowButton({
  label,
  icon,
  disabled,
  onPress,
}: {
  label: string;
  icon: "up" | "down";
  disabled: boolean;
  onPress: () => void;
}) {
  const Glyph = icon === "up" ? ArrowUp : ArrowDown;
  return (
    <Pressable
      accessibilityLabel={label}
      accessibilityRole="button"
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.button,
        disabled && styles.buttonDisabled,
        pressed && styles.pressed,
      ]}
    >
      <Glyph color={disabled ? colors.inkMuted : colors.ink} size={18} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  stack: {
    position: "absolute",
    zIndex: 4,
    alignItems: "center",
    gap: spacing.sm,
  },
  button: {
    width: layout.touchTarget,
    height: layout.touchTarget,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
    borderColor: colors.lineSoft,
    borderRadius: layout.touchTarget / 2,
    backgroundColor: colors.surface,
    shadowColor: colors.inkStrong,
    shadowOpacity: 0.12,
    shadowRadius: 10,
    shadowOffset: { width: 0, height: 3 },
    elevation: 4,
  },
  // Inert, not gone: the pair keeps its size so the enabled button never moves.
  buttonDisabled: {
    backgroundColor: colors.surfaceSubtle,
    shadowOpacity: 0.06,
    elevation: 2,
  },
  pressed: { backgroundColor: colors.surfaceSubtle },
});
