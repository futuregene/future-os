import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useTranslation } from "react-i18next";
import { ArrowLeft, Monitor } from "lucide-react-native";
import { LanguageSettings } from "../../i18n/LanguageSettings";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { DesktopSettings, RemoteBuiltinProvider, RemoteCustomProvider } from "../../remote/types";
import { VERSION } from "../../version.generated";
import { colors, layout, radius, spacing } from "../../theme/tokens";
import { CustomProviderPage } from "./CustomProviderPage";
import { FollowAccountPage } from "./FollowAccountPage";
import { ModelsSettingsPage } from "./ModelsSettingsPage";
import { ProviderKeyPage } from "./ProviderKeyPage";
import { ProvidersSettingsPage } from "./ProvidersSettingsPage";
import { SkillsSettingsPage } from "./SkillsSettingsPage";
import { ResourceStatus, SettingsLink, SettingsSection, SettingsSwitch, settingsStyles } from "./SettingsPrimitives";
import { useDesktopResource } from "./useDesktopResource";

export type SettingsScreenHandle = { goBack(): void };

/**
 * One screen level. Nested provider editors are levels too, so the system back
 * gesture and the header arrow always pop exactly one step — the same rule the
 * desktop dialog's single-level tabs cannot express.
 */
type SettingsRoute =
  | { name: "home" | "preferences" | "models" | "skills" | "language" | "providers" | "followAccount" }
  | { name: "providerKey"; provider: RemoteBuiltinProvider }
  | { name: "providerForm"; provider: RemoteCustomProvider | null };

/** Levels that show the paired-desktop scope banner. */
const SCOPED_LEVELS = new Set(["home", "preferences", "models", "skills", "providers"]);

export function SettingsScreen({ onClose, onCheckUpdate, checkingUpdate, ref }: {
  onClose(): void; onCheckUpdate(): void; checkingUpdate: boolean; ref?: Ref<SettingsScreenHandle>;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const [routes, setRoutes] = useState<SettingsRoute[]>([{ name: "home" }]);
  const page = routes[routes.length - 1]!;
  const push = useCallback((route: SettingsRoute) => setRoutes(current => [...current, route]), []);
  const goBack = useCallback(() => {
    if (routes.length === 1) onClose();
    else setRoutes(current => current.slice(0, -1));
  }, [routes.length, onClose]);
  // A native Modal consumes Android's edge-back before BackHandler. Its owner
  // delegates here so system back and the header pop the same settings level.
  useImperativeHandle(ref, () => ({ goBack }), [goBack]);
  const [saving, setSaving] = useState(false);
  const [failed, setFailed] = useState(false);
  const active = useRef(true);
  const writing = useRef(false);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const supported = remote.capabilities?.has("desktop_settings_v1") ?? false;
  const skillsSupported = remote.capabilities?.has("skill_management_v1") ?? false;
  const providersSupported = remote.capabilities?.has("provider_management_v1") ?? false;
  const enabled = remote.desktopOnline && supported;
  const resource = useDesktopResource(remote.getDesktopSettings, remote.desktopSettingsRevision, enabled);
  const disabled = !enabled || saving || resource.loading || resource.failed || !resource.data;
  const desktop = remote.desktops.find(item => item.pairId === remote.credentials?.pairId);
  const desktopName = desktop?.name || remote.credentials?.expectedDesktopId || desktop?.desktopId || t("desktops.title");
  // Provider pages read and write their own authoritative snapshot; the shared
  // settings revision refetches them whenever the desktop changes a provider.
  const goBackFromEditor = useCallback(() => {
    setRoutes(current => current.slice(0, -1));
  }, []);

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

  // Each level renders on its own; the stack keeps only the visible one mounted,
  // so returning to the provider list always re-reads the desktop's snapshot.
  function renderLevel() {
    switch (page.name) {
      case "providers":
        return <ProvidersSettingsPage
          onOpenBuiltin={provider => push({ name: "providerKey", provider })}
          onOpenCustom={provider => push({ name: "providerForm", provider })}
        />;
      case "providerKey":
        return <ProviderKeyPage key={page.provider.id} provider={page.provider} onSaved={goBackFromEditor} />;
      case "providerForm":
        return <CustomProviderPage key={page.provider?.id ?? "new"} onDone={goBackFromEditor} provider={page.provider} />;
      case "models":
        return enabled
          ? <ModelsSettingsPage settings={resource.data} disabled={disabled} onChange={patch => void changeSettings(patch)} />
          : null;
      case "skills":
        return remote.desktopOnline && skillsSupported ? <SkillsSettingsPage /> : null;
      case "language":
        return <ScrollView contentContainerStyle={settingsStyles.content}>
          <SettingsSection title={t("desktopSettings.thisPhone")}><View style={settingsStyles.card}><LanguageSettings /></View></SettingsSection>
        </ScrollView>;
      // Not scoped to the paired desktop: following the account needs no
      // connection, so this level stays usable while the desktop is offline.
      case "followAccount":
        return <FollowAccountPage />;
      case "preferences":
        return <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
          <SettingsSection title={t("desktopSettings.automation")}>
            {remote.desktopOnline && !supported ? <Text style={settingsStyles.description}>{t("desktopSettings.updateDesktop")}</Text> : null}
            <SettingsSwitch label={t("desktopSettings.autoUpgradeSkills")} description={t("desktopSettings.autoUpgradeSkillsHint")}
              value={resource.data?.autoUpgradeSkills ?? false} disabled={disabled} onChange={autoUpgradeSkills => void changeSettings({ autoUpgradeSkills })} />
            <SettingsSwitch label={t("desktopSettings.autoTitleFirstTurn")} description={t("desktopSettings.autoTitleFirstTurnHint")}
              value={resource.data?.autoTitleFirstTurn ?? false} disabled={disabled} onChange={autoTitleFirstTurn => void changeSettings({ autoTitleFirstTurn })} />
            {/* Default on when the desktop does not report it (an older desktop
                predates the toggle), matching the desktop's own default. */}
            <SettingsSwitch label={t("desktopSettings.skillRecommend")} description={t("desktopSettings.skillRecommendHint")}
              value={resource.data?.skillRecommend ?? true} disabled={disabled} onChange={skillRecommend => void changeSettings({ skillRecommend })} />
            <SettingsSwitch label={t("desktopSettings.autoConnectRemote")} description={t("desktopSettings.autoConnectRemoteHint")}
              value={resource.data?.autoConnectRemote ?? false} disabled={disabled} onChange={autoConnectRemote => void changeSettings({ autoConnectRemote })} />
            {enabled ? <ResourceStatus loading={resource.loading || saving} failed={resource.failed} onReload={() => void resource.reload()} /> : null}
          </SettingsSection>
          <SettingsSection title={t("approvalTier.title")}>
            <View accessibilityRole="radiogroup" style={settingsStyles.actions}>
              {(["manual", "sandbox", "off"] as const).filter(tier => tier !== "sandbox" || remote.sandboxAvailable).map(tier =>
                <Pressable key={tier} accessibilityRole="radio" accessibilityLabel={t(`approvalTier.${tier}`)}
                  accessibilityState={{ checked: remote.approvalTier === tier, disabled: !remote.desktopOnline || saving }}
                  disabled={!remote.desktopOnline || saving} onPress={() => void selectApproval(tier)}
                  style={[styles.approval, remote.approvalTier === tier && styles.approvalSelected]}>
                  <Text style={settingsStyles.label}>{t(`approvalTier.${tier}`)}</Text>
                </Pressable>)}
            </View>
          </SettingsSection>
        </ScrollView>;
      default:
        return <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
          <SettingsSection title={t("desktopSettings.currentDesktop")}>
            <SettingsLink label={t("desktopSettings.preferences")} onPress={() => push({ name: "preferences" })} />
            <SettingsLink label={t("desktopSettings.models")} disabled={!enabled} onPress={() => push({ name: "models" })} />
            <SettingsLink label={t("desktopSettings.providers")} disabled={!remote.desktopOnline || !providersSupported} onPress={() => push({ name: "providers" })} />
            <SettingsLink label={t("desktopSettings.skills")} disabled={!remote.desktopOnline || !skillsSupported} onPress={() => push({ name: "skills" })} />
            {remote.desktopOnline && (!supported || !skillsSupported || !providersSupported) ? <Text style={settingsStyles.description}>{t("desktopSettings.updateDesktop")}</Text> : null}
          </SettingsSection>
          <SettingsSection title={t("desktopSettings.thisPhone")}>
            <SettingsLink label={t("language.title")} onPress={() => push({ name: "language" })} />
            {/* Deliberately never disabled: following the official account needs no
                desktop connection, unlike every other row on this screen. */}
            <SettingsLink label={t("desktopSettings.followAccount")} onPress={() => push({ name: "followAccount" })} />
            <SettingsLink label={t("update.check")} disabled={checkingUpdate} loading={checkingUpdate} onPress={onCheckUpdate} />
          </SettingsSection>
          <Text style={styles.version}>{t("common.version", { version: VERSION })}</Text>
        </ScrollView>;
    }
  }

  return <SafeAreaView style={settingsStyles.page} onAccessibilityEscape={goBack}>
    <View style={styles.column}>
    <View style={styles.header}>
      <Pressable accessibilityLabel={t("common.back")} accessibilityRole="button"
        onPress={goBack}
        style={({ pressed }) => [styles.headerButton, pressed && styles.pressed]}>
        <ArrowLeft size={22} color={colors.ink} />
      </Pressable>
      <Text accessibilityRole="header" style={styles.title}>{page.name === "home" ? t("sessions.settings") : t(`desktopSettings.${page.name}`)}</Text>
    </View>
    {SCOPED_LEVELS.has(page.name) ? <View style={styles.deviceScope}>
      <View style={styles.deviceIcon}><Monitor size={20} color={colors.accent} /></View>
      <View style={styles.heading}>
        <Text style={styles.deviceName}>{t("desktopSettings.boundDesktop", { name: desktopName })}</Text>
        <Text style={settingsStyles.description}>{t("desktopSettings.sharedHint")}</Text>
      </View>
    </View> : null}
    {failed ? <Text accessibilityRole="alert" style={[styles.notice, settingsStyles.error]}>{t("desktopSettings.saveFailed")}</Text> : null}
    {page.name === "models" && enabled && (resource.failed || resource.loading) ? <View style={styles.notice}>
      <ResourceStatus loading={resource.loading} failed={resource.failed} onReload={() => void resource.reload()} />
    </View> : null}
    {!remote.desktopOnline ? <Text style={styles.notice}>{t("desktopSettings.offline")}</Text> : null}
    {renderLevel()}
    </View>
  </SafeAreaView>;
}

const styles = StyleSheet.create({
  column: { flex: 1, width: "100%", maxWidth: layout.formMaxWidth, alignSelf: "center", paddingTop: spacing.sm },
  header: { flexDirection: "row", alignItems: "center", gap: spacing.sm, minHeight: 56, paddingHorizontal: layout.gutter, marginBottom: spacing.md },
  heading: { flex: 1, minWidth: 0 },
  title: { flex: 1, color: colors.inkStrong, fontSize: 22, fontWeight: "700" },
  headerButton: { width: layout.touchTarget, height: layout.touchTarget, borderRadius: radius.md, alignItems: "center", justifyContent: "center" },
  pressed: { backgroundColor: colors.surfaceSubtle },
  deviceScope: { flexDirection: "row", alignItems: "flex-start", gap: spacing.md, marginHorizontal: layout.gutter, padding: spacing.md, borderRadius: radius.lg, backgroundColor: colors.accentSoft },
  deviceIcon: { width: 36, height: 36, borderRadius: radius.md, alignItems: "center", justifyContent: "center", backgroundColor: colors.surface },
  deviceName: { color: colors.inkStrong, fontSize: 14, fontWeight: "600", marginBottom: spacing.xs },
  version: { ...settingsStyles.description, textAlign: "center" },
  notice: { color: colors.inkMuted, padding: layout.gutter, fontSize: 13 },
  approval: { minHeight: layout.touchTarget, justifyContent: "center", padding: spacing.sm, borderWidth: 1, borderColor: colors.line, borderRadius: radius.md, backgroundColor: colors.surface },
  approvalSelected: { borderColor: colors.accent, backgroundColor: colors.accentSoft },
});
