import type { ReactNode } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";
import { colors, radius, spacing } from "../theme/tokens";

interface ButtonProps {
  label: string;
  icon?: ReactNode;
  variant?: "primary" | "secondary" | "danger" | "ghost";
  disabled?: boolean;
  loading?: boolean;
  compact?: boolean;
  onPress(): void;
}

export function Button({
  label,
  icon,
  variant = "primary",
  disabled = false,
  loading = false,
  compact = false,
  onPress,
}: ButtonProps) {
  const inactive = disabled || loading;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      disabled={inactive}
      onPress={onPress}
      style={({ pressed }) => [
        styles.base,
        compact && styles.compact,
        styles[variant],
        pressed && !inactive && styles.pressed,
        inactive && styles.disabled,
      ]}
    >
      {loading ? (
        <ActivityIndicator color={variant === "primary" ? colors.surface : colors.accent} />
      ) : (
        <View style={styles.content}>
          {icon}
          <Text style={[styles.label, styles[`${variant}Label`]]}>{label}</Text>
        </View>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  base: {
    minHeight: 48,
    paddingHorizontal: spacing.lg,
    borderRadius: radius.md,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: 1,
  },
  compact: {
    minHeight: 38,
    paddingHorizontal: spacing.md,
  },
  primary: {
    backgroundColor: colors.accent,
    borderColor: colors.accent,
  },
  secondary: {
    backgroundColor: colors.surface,
    borderColor: colors.line,
  },
  danger: {
    backgroundColor: colors.dangerSoft,
    borderColor: colors.dangerLine,
  },
  ghost: {
    backgroundColor: "transparent",
    borderColor: "transparent",
  },
  content: {
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.sm,
  },
  label: {
    fontSize: 15,
    fontWeight: "600",
  },
  primaryLabel: {
    color: colors.surface,
  },
  secondaryLabel: {
    color: colors.ink,
  },
  dangerLabel: {
    color: colors.danger,
  },
  ghostLabel: {
    color: colors.inkSoft,
  },
  pressed: {
    opacity: 0.78,
  },
  disabled: {
    opacity: 0.5,
  },
});
