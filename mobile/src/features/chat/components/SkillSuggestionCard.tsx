import { Lightbulb, X } from "lucide-react-native";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";
import type { TFunction } from "i18next";
import { colors, radius, spacing } from "../../../theme/tokens";
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

const styles = StyleSheet.create({
  card: {
    backgroundColor: colors.accentSoft,
    borderColor: colors.focus,
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
    color: colors.inkSoft,
    fontSize: 12,
    fontWeight: "600",
  },
  skill: {
    color: colors.inkStrong,
    flexShrink: 1,
    fontFamily: "Menlo",
    fontSize: 12,
    fontWeight: "600",
  },
  description: {
    color: colors.inkSoft,
    fontSize: 12,
    lineHeight: 17,
  },
  actions: {
    flexDirection: "row",
    gap: spacing.sm,
    marginTop: spacing.xs,
  },
  primary: {
    alignItems: "center",
    backgroundColor: colors.accent,
    borderRadius: radius.sm,
    justifyContent: "center",
    minHeight: 32,
    paddingHorizontal: spacing.md,
  },
  primaryPressed: {
    backgroundColor: colors.accentHover,
  },
  primaryLabel: {
    color: colors.surface,
    fontSize: 13,
    fontWeight: "600",
  },
  secondary: {
    alignItems: "center",
    borderColor: colors.line,
    borderRadius: radius.sm,
    borderWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    gap: spacing.xs,
    minHeight: 32,
    paddingHorizontal: spacing.md,
  },
  secondaryPressed: {
    backgroundColor: colors.surfaceSubtle,
  },
  secondaryLabel: {
    color: colors.inkSoft,
    fontSize: 13,
  },
  disabled: {
    opacity: 0.6,
  },
});
