import { useEffect, useRef, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useTranslation } from "react-i18next";
import { ChevronLeft, X } from "lucide-react-native";
import { Button } from "../../components/Button";
import { LanguageSettings } from "../../i18n/LanguageSettings";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { DesktopSettings } from "../../remote/types";
import { VERSION } from "../../version.generated";
import { colors, layout, spacing } from "../../theme/tokens";
import { ModelsSettingsPage } from "./ModelsSettingsPage";
import { SkillsSettingsPage } from "./SkillsSettingsPage";
import { ResourceStatus, SettingsLink, SettingsSection, SettingsSwitch, settingsStyles } from "./SettingsPrimitives";
import { useDesktopResource } from "./useDesktopResource";

export function SettingsScreen({ onClose, onManageDesktops, onCheckUpdate, onUnpair, checkingUpdate }: {
  onClose(): void; onManageDesktops(): void; onCheckUpdate(): void; onUnpair(): void; checkingUpdate: boolean;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const [page, setPage] = useState<"home" | "models" | "skills">("home");
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  const active = useRef(true);
  const writing = useRef(false);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const supported = remote.capabilities?.has("desktop_settings_v1") ?? false;
  const skillsSupported = remote.capabilities?.has("skill_management_v1") ?? false;
  const enabled = remote.desktopOnline && supported;
  const resource = useDesktopResource(remote.getDesktopSettings, remote.desktopSettingsRevision, enabled);
  const disabled = !enabled || saving || resource.loading || resource.failed || !resource.data;
  const desktop = remote.desktops.find(item => item.pairId === remote.credentials?.pairId);
  const desktopName = desktop?.name || remote.credentials?.expectedDesktopId || desktop?.desktopId || t("desktops.title");

  const changeSettings = async (patch: Partial<DesktopSettings>) => {
    if (disabled || writing.current || !active.current) return;
    writing.current = true;
    setSaving(true);
    setFailed(false);
    try {
      await remote.updateDesktopSettings(patch);
    } catch {
      if (active.current) setFailed(true);
    } finally {
      writing.current = false;
      if (active.current) {
        // Read back even on failure: a lost reply can follow a successful save.
        await resource.reload();
        if (active.current) setSaving(false);
      }
    }
  };
  const selectApproval = async (tier: string) => {
    if (!remote.desktopOnline || saving || writing.current) return;
    writing.current = true;
    setSaving(true);
    setFailed(false);
    try { await remote.setApprovalTier(tier); }
    catch { if (active.current) setFailed(true); }
    finally { writing.current = false; if (active.current) setSaving(false); }
  };

  return <SafeAreaView style={settingsStyles.page}>
    <View style={styles.header}>
      {page !== "home" ? <Pressable accessibilityLabel={t("common.back")} accessibilityRole="button" onPress={() => setPage("home")} style={styles.headerButton}>
        <ChevronLeft size={22} color={colors.ink} />
      </Pressable> : null}
      <View style={styles.heading}>
        <Text accessibilityRole="header" style={styles.title}>{page === "home" ? t("sessions.settings") : t(`desktopSettings.${page}`)}</Text>
        <Text numberOfLines={1} style={settingsStyles.description}>{desktopName}</Text>
      </View>
      <Pressable accessibilityLabel={t("common.close")} accessibilityRole="button" onPress={onClose} style={styles.headerButton}>
        <X size={22} color={colors.inkMuted} />
      </Pressable>
    </View>
    {failed ? <Text accessibilityRole="alert" style={[styles.notice, settingsStyles.error]}>{t("desktopSettings.saveFailed")}</Text> : null}
    {page === "models" && enabled && (resource.failed || resource.loading) ? <View style={styles.notice}>
      <ResourceStatus loading={resource.loading} failed={resource.failed} onReload={() => void resource.reload()} />
    </View> : null}
    {!remote.desktopOnline ? <Text style={styles.notice}>{t("desktopSettings.offline")}</Text> : null}
    {page === "models" && enabled ? <ModelsSettingsPage settings={resource.data} disabled={disabled} onChange={patch => void changeSettings(patch)} />
      : page === "skills" && remote.desktopOnline && skillsSupported ? <SkillsSettingsPage />
      : <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
        <SettingsSection title={t("desktopSettings.currentDesktop")}>
          <Text style={settingsStyles.description}>{t("desktopSettings.sharedHint")}</Text>
          {remote.desktopOnline && !supported ? <Text style={settingsStyles.description}>{t("desktopSettings.updateDesktop")}</Text> : null}
          <SettingsSwitch label={t("desktopSettings.autoUpgradeSkills")} description={t("desktopSettings.autoUpgradeSkillsHint")}
            value={resource.data?.autoUpgradeSkills ?? false} disabled={disabled} onChange={autoUpgradeSkills => void changeSettings({ autoUpgradeSkills })} />
          <SettingsSwitch label={t("desktopSettings.autoTitleFirstTurn")} description={t("desktopSettings.autoTitleFirstTurnHint")}
            value={resource.data?.autoTitleFirstTurn ?? false} disabled={disabled} onChange={autoTitleFirstTurn => void changeSettings({ autoTitleFirstTurn })} />
          <SettingsSwitch label={t("desktopSettings.autoConnectRemote")} description={t("desktopSettings.autoConnectRemoteHint")}
            value={resource.data?.autoConnectRemote ?? false} disabled={disabled} onChange={autoConnectRemote => void changeSettings({ autoConnectRemote })} />
          {enabled ? <ResourceStatus loading={resource.loading || saving} failed={resource.failed} onReload={() => void resource.reload()} /> : null}
          <Text style={settingsStyles.label}>{t("approvalTier.title")}</Text>
          <View style={settingsStyles.actions}>
            {(["manual", "sandbox", "off"] as const).filter(tier => tier !== "sandbox" || remote.sandboxAvailable).map(tier =>
              <Pressable key={tier} accessibilityRole="radio" accessibilityLabel={t(`approvalTier.${tier}`)}
                accessibilityState={{ checked: remote.approvalTier === tier, disabled: !remote.desktopOnline || saving }}
                disabled={!remote.desktopOnline || saving} onPress={() => void selectApproval(tier)}
                style={[styles.approval, remote.approvalTier === tier && styles.approvalSelected]}>
                <Text style={settingsStyles.label}>{t(`approvalTier.${tier}`)}</Text>
              </Pressable>)}
          </View>
        </SettingsSection>
        <SettingsSection title={t("desktopSettings.modelsAndSkills")}>
          <SettingsLink label={t("desktopSettings.models")} disabled={!enabled} onPress={() => setPage("models")} />
          <SettingsLink label={t("desktopSettings.skills")} disabled={!remote.desktopOnline || !skillsSupported} onPress={() => setPage("skills")} />
          {remote.desktopOnline && !skillsSupported ? <Text style={settingsStyles.description}>{t("desktopSettings.updateDesktop")}</Text> : null}
        </SettingsSection>
        <SettingsSection title={t("desktopSettings.thisPhone")}>
          <LanguageSettings />
          <Text style={settingsStyles.description}>{t("common.version", { version: VERSION })}</Text>
          <Button label={t("update.check")} loading={checkingUpdate} onPress={onCheckUpdate} variant="secondary" />
          <Button label={t("desktops.title")} onPress={onManageDesktops} variant="secondary" />
          <Button label={t("sessions.unpair")} onPress={onUnpair} variant="danger" />
        </SettingsSection>
      </ScrollView>}
  </SafeAreaView>;
}

const styles = StyleSheet.create({
  header: { flexDirection: "row", alignItems: "center", gap: spacing.sm, paddingHorizontal: layout.gutter, paddingVertical: spacing.sm, borderBottomWidth: StyleSheet.hairlineWidth, borderBottomColor: colors.line },
  heading: { flex: 1, minWidth: 0 },
  title: { color: colors.ink, fontSize: 20, fontWeight: "600" },
  headerButton: { minWidth: layout.touchTarget, minHeight: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  notice: { color: colors.inkMuted, padding: layout.gutter, fontSize: 13 },
  approval: { minHeight: layout.touchTarget, justifyContent: "center", padding: spacing.sm, borderWidth: 1, borderColor: colors.line, borderRadius: 8 },
  approvalSelected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
});
