import type { ReactNode } from "react";
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  Text,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { layout, radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";

export function FloatingTimelineButton({
  label,
  icon,
  busy = false,
  onPress,
  style,
}: {
  label: string;
  icon: ReactNode;
  busy?: boolean;
  onPress: () => void;
  style: StyleProp<ViewStyle>;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityLabel={label}
      accessibilityRole="button"
      accessibilityState={{ busy, disabled: busy }}
      disabled={busy}
      onPress={onPress}
      style={({ pressed }) => [styles.button, style, pressed && styles.pressed]}
    >
      {busy ? <ActivityIndicator color={colors.inkSoft} size="small" /> : icon}
      <Text style={styles.label}>{label}</Text>
    </Pressable>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  button: {
    position: "absolute",
    alignSelf: "center",
    zIndex: 3,
    minHeight: layout.touchTarget,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.xs,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderWidth: 1,
    borderColor: theme.colors.lineSoft,
    borderRadius: radius.pill,
    backgroundColor: theme.colors.surface,
    shadowColor: theme.colors.inkStrong,
    shadowOpacity: 0.08,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 3 },
    elevation: 3,
  },
  pressed: { opacity: 0.72 },
  label: { color: theme.colors.inkSoft, fontSize: theme.font(13), fontWeight: "600" },
}));
