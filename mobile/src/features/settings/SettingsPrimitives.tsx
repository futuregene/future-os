import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react-native";
import { Pressable, StyleSheet, Switch, Text, View } from "react-native";
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

export function SettingsLink({ label, disabled = false, onPress }: { label: string; disabled?: boolean; onPress(): void }) {
  return <Pressable accessibilityRole="button" accessibilityLabel={label} accessibilityState={{ disabled }}
    disabled={disabled} onPress={onPress} style={[settingsStyles.row, disabled && { opacity: 0.5 }]}>
    <Text style={settingsStyles.label}>{label}</Text>
    <ChevronRight size={18} color={colors.inkMuted} />
  </Pressable>;
}

export function ResourceStatus({ loading, failed, onReload }: { loading: boolean; failed: boolean; onReload(): void }) {
  const { t } = useTranslation();
  return <View style={settingsStyles.status}>
    {failed ? <Text accessibilityRole="alert" style={settingsStyles.error}>{t("desktopSettings.loadFailed")}</Text> : null}
    <Button compact label={t("desktopSettings.refresh")} loading={loading} onPress={onReload} variant="secondary" />
  </View>;
}

export const settingsStyles = StyleSheet.create({
  page: { flex: 1, backgroundColor: colors.surface },
  content: { padding: layout.gutter, gap: spacing.lg, width: "100%", maxWidth: layout.contentMaxWidth, alignSelf: "center" },
  section: { gap: spacing.sm },
  sectionTitle: { color: colors.inkMuted, fontSize: 13, fontWeight: "600", marginTop: spacing.sm },
  row: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: spacing.md, minHeight: 52, paddingVertical: spacing.sm, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: colors.line },
  labelContainer: { flex: 1, minWidth: 0, gap: spacing.xs },
  label: { color: colors.ink, fontSize: 15, flexShrink: 1 },
  description: { color: colors.inkMuted, fontSize: 13, lineHeight: 19 },
  error: { color: colors.danger, fontSize: 13 },
  status: { gap: spacing.sm, alignItems: "flex-start" },
  search: { borderWidth: 1, borderColor: colors.line, borderRadius: radius.md, color: colors.ink, fontSize: 15, padding: spacing.md, minHeight: layout.touchTarget },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: spacing.sm },
});
