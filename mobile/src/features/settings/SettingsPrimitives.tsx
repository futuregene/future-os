import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react-native";
import { ActivityIndicator, Pressable, StyleSheet, Switch, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { layout, radius, spacing } from "../../theme/tokens";
import type { Theme } from "../../theme/theme";
import { createThemedStyles, useTheme } from "../../theme/ThemeContext";

export function SettingsSection({ title, children }: { title: string; children: ReactNode }) {
  const settingsStyles = useSettingsStyles();
  return <View style={settingsStyles.section}>
    <Text accessibilityRole="header" style={settingsStyles.sectionTitle}>{title}</Text>
    {children}
  </View>;
}

export function SettingsSwitch({ label, description, value, disabled = false, onChange }: {
  label: string; description?: string; value: boolean; disabled?: boolean; onChange(value: boolean): void;
}) {
  const settingsStyles = useSettingsStyles();
  const { colors } = useTheme();
  return <View style={settingsStyles.row}>
    <View style={settingsStyles.labelContainer}>
      <Text style={settingsStyles.label}>{label}</Text>
      {description ? <Text style={settingsStyles.description}>{description}</Text> : null}
    </View>
    <Switch accessibilityLabel={label} disabled={disabled} value={value} onValueChange={onChange}
      trackColor={{ false: colors.line, true: colors.accent }} />
  </View>;
}

export function SettingsLink({ label, description, disabled = false, loading = false, onPress }: {
  label: string; description?: string; disabled?: boolean; loading?: boolean; onPress(): void;
}) {
  const settingsStyles = useSettingsStyles();
  const { colors } = useTheme();
  return <Pressable accessibilityRole="button" accessibilityLabel={label} accessibilityState={{ disabled: disabled || loading, busy: loading }}
    disabled={disabled || loading} onPress={onPress} style={({ pressed }) => [settingsStyles.row, pressed && settingsStyles.pressed, disabled && { opacity: 0.5 }]}>
    <View style={settingsStyles.labelContainer}>
      <Text style={settingsStyles.label}>{label}</Text>
      {description ? <Text style={settingsStyles.description}>{description}</Text> : null}
    </View>
    {loading ? <ActivityIndicator size="small" color={colors.accent} /> : <ChevronRight size={18} color={colors.inkMuted} />}
  </Pressable>;
}

/** A labeled form control on the settings surface (label, hint, then the input). */
export function SettingsField({ label, hint, error, children }: {
  label: string; hint?: string; error?: string; children: ReactNode;
}) {
  const settingsStyles = useSettingsStyles();
  return <View style={settingsStyles.field}>
    <Text style={settingsStyles.fieldLabel}>{label}</Text>
    {children}
    {error ? <Text accessibilityRole="alert" style={settingsStyles.error}>{error}</Text> : hint ? <Text style={settingsStyles.description}>{hint}</Text> : null}
  </View>;
}

export function ResourceStatus({ loading, failed, onReload }: { loading: boolean; failed: boolean; onReload(): void }) {
  const settingsStyles = useSettingsStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  if (!loading && !failed) return null;
  return <View style={settingsStyles.status}>
    {loading ? <ActivityIndicator size="small" color={colors.accent} /> : null}
    {failed ? <>
      <Text accessibilityRole="alert" style={settingsStyles.error}>{t("desktopSettings.loadFailed")}</Text>
      <Button compact label={t("common.retry")} loading={loading} onPress={onReload} variant="secondary" />
    </> : null}
  </View>;
}

/**
 * The settings sheet, as a factory as well as a hook: a page whose own sheet has
 * a style that *is* one of these (`SettingsScreen`'s centered version footer)
 * spreads this one's entry rather than restating its colours.
 */
export const createSettingsStyles = (theme: Theme) => StyleSheet.create({
  page: { flex: 1, backgroundColor: theme.colors.canvas },
  content: { padding: layout.gutter, gap: spacing.md, width: "100%", maxWidth: layout.formMaxWidth, alignSelf: "center" },
  section: { gap: spacing.sm },
  sectionTitle: { color: theme.colors.inkMuted, fontSize: theme.font(13), fontWeight: "600", marginTop: spacing.sm },
  card: { padding: spacing.md, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.lg, backgroundColor: theme.colors.surface },
  row: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: spacing.md, minHeight: 52, padding: spacing.md, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.lg, backgroundColor: theme.colors.surface },
  pressed: { backgroundColor: theme.colors.surfaceSubtle },
  labelContainer: { flex: 1, minWidth: 0, gap: spacing.xs },
  label: { color: theme.colors.ink, fontSize: theme.font(15), flexShrink: 1 },
  description: { color: theme.colors.inkMuted, fontSize: theme.font(13), lineHeight: theme.leading(19) },
  error: { color: theme.colors.danger, fontSize: theme.font(13) },
  status: { gap: spacing.sm, alignItems: "flex-start" },
  search: { borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.md, backgroundColor: theme.colors.surface, color: theme.colors.ink, fontSize: theme.font(15), padding: spacing.md, minHeight: layout.touchTarget },
  input: { borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.md, backgroundColor: theme.colors.surface, color: theme.colors.ink, fontSize: theme.font(15), padding: spacing.md, minHeight: layout.touchTarget },
  inputDisabled: { backgroundColor: theme.colors.surfaceSubtle, color: theme.colors.inkMuted },
  field: { gap: spacing.sm },
  fieldLabel: { color: theme.colors.inkMuted, fontSize: theme.font(13), fontWeight: "600" },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: spacing.sm },
});

export const useSettingsStyles = createThemedStyles(createSettingsStyles);
