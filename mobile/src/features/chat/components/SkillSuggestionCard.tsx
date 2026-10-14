import { Lightbulb, X } from "lucide-react-native";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";
import type { TFunction } from "i18next";
import { radius, spacing } from "../../../theme/tokens";
import { createThemedStyles, useTheme } from "../../../theme/ThemeContext";
import type { PendingSuggestion } from "../useSkillRecommendation";

/**
 * The docked skill suggestion above the composer (PRD v1.6 §6.1).
 *
 * One skill, its summary, and the two decisions: install and use it, or send the
 * held message without it. The message is not sent while this is on screen, so
 * dismissing means "send what I typed" rather than "cancel".
 */
export function SkillSuggestionCard({
  t,
  suggestion,
  installing,
  onInstall,
  onDismiss,
}: {
  t: TFunction;
  suggestion: PendingSuggestion;
  installing: boolean;
  onInstall: () => void;
  onDismiss: () => void;
}) {
  const styles = useStyles();
  const { colors } = useTheme();
  return (
    <View accessibilityRole="summary" style={styles.card}>
      <View style={styles.heading}>
        <Lightbulb color={colors.accent} size={15} />
        <Text style={styles.title}>{t("chat.skillRecommend")}</Text>
        <Text numberOfLines={1} style={styles.skill}>
          /{suggestion.skill.name}
        </Text>
      </View>
      {suggestion.skill.description ? (
        <Text numberOfLines={3} style={styles.description}>
          {suggestion.skill.description}
        </Text>
      ) : null}
      <View style={styles.actions}>
        <Pressable
          accessibilityRole="button"
          accessibilityState={{ disabled: installing, busy: installing }}
          disabled={installing}
          onPress={onInstall}
          style={({ pressed }) => [
            styles.primary,
            pressed && !installing ? styles.primaryPressed : null,
            installing ? styles.disabled : null,
          ]}
        >
          {installing ? (
            <ActivityIndicator color={colors.surface} size="small" />
          ) : (
            <Text style={styles.primaryLabel}>{t("chat.skillInstallAndUse")}</Text>
          )}
        </Pressable>
        <Pressable
          accessibilityRole="button"
          accessibilityState={{ disabled: installing }}
          disabled={installing}
          onPress={onDismiss}
          style={({ pressed }) => [
            styles.secondary,
            pressed && !installing ? styles.secondaryPressed : null,
            installing ? styles.disabled : null,
          ]}
        >
          <X color={colors.inkSoft} size={14} />
          <Text style={styles.secondaryLabel}>{t("chat.skillDismissAndSend")}</Text>
        </Pressable>
      </View>
    </View>
  );
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  card: {
    backgroundColor: theme.colors.accentSoft,
    borderColor: theme.colors.focus,
    borderRadius: radius.md,
    borderWidth: StyleSheet.hairlineWidth,
    gap: spacing.xs,
    marginHorizontal: spacing.md,
    marginBottom: spacing.sm,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
  },
  heading: {
    alignItems: "center",
    flexDirection: "row",
    gap: spacing.xs,
  },
  title: {
    color: theme.colors.inkSoft,
    fontSize: theme.font(12),
    fontWeight: "600",
  },
  skill: {
    color: theme.colors.inkStrong,
    flexShrink: 1,
    fontFamily: "Menlo",
    fontSize: theme.font(12),
    fontWeight: "600",
  },
  description: {
    color: theme.colors.inkSoft,
    fontSize: theme.font(12),
    lineHeight: theme.leading(17),
  },
  actions: {
    flexDirection: "row",
    gap: spacing.sm,
    marginTop: spacing.xs,
  },
  primary: {
    alignItems: "center",
    backgroundColor: theme.colors.accent,
    borderRadius: radius.sm,
    justifyContent: "center",
    minHeight: 32,
    paddingHorizontal: spacing.md,
  },
  primaryPressed: {
    backgroundColor: theme.colors.accentHover,
  },
  primaryLabel: {
    color: theme.colors.surface,
    fontSize: theme.font(13),
    fontWeight: "600",
  },
  secondary: {
    alignItems: "center",
    borderColor: theme.colors.line,
    borderRadius: radius.sm,
    borderWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    gap: spacing.xs,
    minHeight: 32,
    paddingHorizontal: spacing.md,
  },
  secondaryPressed: {
    backgroundColor: theme.colors.surfaceSubtle,
  },
  secondaryLabel: {
    color: theme.colors.inkSoft,
    fontSize: theme.font(13),
  },
  disabled: {
    opacity: 0.6,
  },
}));
