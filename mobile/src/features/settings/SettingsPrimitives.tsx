import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react-native";
import { ActivityIndicator, Pressable, StyleSheet, Switch, Text, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { colors, layout, radius, spacing } from "../../theme/tokens";

export function SettingsSection({ title, children }: { title: string; children: ReactNode }) {
  return <View style={settingsStyles.section}>
    <Text accessibilityRole="header" style={settingsStyles.sectionTitle}>{title}</Text>
    {children}
  </View>;
}

export function SettingsSwitch({ label, description, value, disabled, onChange }: {
  label: string; description?: string; value: boolean; disabled: boolean; onChange(value: boolean): void;
}) {
  return <View style={settingsStyles.row}>
    <View style={settingsStyles.labelContainer}>
      <Text style={settingsStyles.label}>{label}</Text>
      {description ? <Text style={settingsStyles.description}>{description}</Text> : null}
    </View>
    <Switch accessibilityLabel={label} disabled={disabled} value={value} onValueChange={onChange}
      trackColor={{ false: colors.line, true: colors.accent }} />
  </View>;
}

export function SettingsLink({ label, disabled = false, loading = false, onPress }: {
  label: string; disabled?: boolean; loading?: boolean; onPress(): void;
}) {
  return <Pressable accessibilityRole="button" accessibilityLabel={label} accessibilityState={{ disabled: disabled || loading, busy: loading }}
    disabled={disabled || loading} onPress={onPress} style={({ pressed }) => [settingsStyles.row, pressed && settingsStyles.pressed, disabled && { opacity: 0.5 }]}>
    <Text style={settingsStyles.label}>{label}</Text>
    {loading ? <ActivityIndicator size="small" color={colors.accent} /> : <ChevronRight size={18} color={colors.inkMuted} />}
  </Pressable>;
}

export function ResourceStatus({ loading, failed, onReload }: { loading: boolean; failed: boolean; onReload(): void }) {
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

export const settingsStyles = StyleSheet.create({
  page: { flex: 1, backgroundColor: colors.canvas },
  content: { padding: layout.gutter, gap: spacing.md, width: "100%", maxWidth: layout.formMaxWidth, alignSelf: "center" },
  section: { gap: spacing.sm },
  sectionTitle: { color: colors.inkMuted, fontSize: 13, fontWeight: "600", marginTop: spacing.sm },
  card: { padding: spacing.md, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg, backgroundColor: colors.surface },
  row: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: spacing.md, minHeight: 52, padding: spacing.md, borderWidth: 1, borderColor: colors.line, borderRadius: radius.lg, backgroundColor: colors.surface },
  pressed: { backgroundColor: colors.surfaceSubtle },
  labelContainer: { flex: 1, minWidth: 0, gap: spacing.xs },
  label: { color: colors.ink, fontSize: 15, flexShrink: 1 },
  description: { color: colors.inkMuted, fontSize: 13, lineHeight: 19 },
  error: { color: colors.danger, fontSize: 13 },
  status: { gap: spacing.sm, alignItems: "flex-start" },
  search: { borderWidth: 1, borderColor: colors.line, borderRadius: radius.md, backgroundColor: colors.surface, color: colors.ink, fontSize: 15, padding: spacing.md, minHeight: layout.touchTarget },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: spacing.sm },
});
