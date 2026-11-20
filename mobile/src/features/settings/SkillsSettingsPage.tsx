import { useEffect, useRef, useState } from "react";
import { FlatList, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { AvailableSkill, DesktopSettings, InstalledSkill } from "../../remote/types";
import { useTheme } from "../../theme/ThemeContext";
import { ResourceStatus, SettingsSwitch, useSettingsStyles } from "./SettingsPrimitives";
import { isSkillUpgrade } from "./skillVersion";
import { useDesktopResource } from "./useDesktopResource";

export function SkillsSettingsPage({
  settings,
  disabled: settingsDisabled = false,
  onChange,
}: {
  /** The desktop's settings snapshot, for the two automation switches above the list. */
  settings?: DesktopSettings | null;
  disabled?: boolean;
  onChange?: (patch: Partial<DesktopSettings>) => void;
} = {}) {
  const settingsStyles = useSettingsStyles();
  const { colors } = useTheme();
  const { t, i18n } = useTranslation();
  const remote = useRemoteControls();
  // Reads, not controls: a standalone machine's switch marks the page busy,
  // while a machine another device drives refuses the read itself.
  const installed = useDesktopResource(remote.listInstalledSkills, remote.skillsRevision, remote.desktopReadable);
  const available = useDesktopResource(remote.listAvailableSkills, 0, remote.desktopReadable);
  const [tab, setTab] = useState<"installed" | "available">("installed");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);
  const active = useRef(true);
  const writing = useRef(false);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const installedById = new Map((installed.data ?? []).map(skill => [skill.id, skill]));
  const availableById = new Map((available.data ?? []).map(skill => [skill.id, skill]));
  const upgrades = (available.data ?? []).filter(skill => isSkillUpgrade(installedById.get(skill.id)?.version, skill.latestVersion));
  const disabled = busy || !remote.desktopOnline || installed.loading || installed.failed || !installed.data;
  const chinese = i18n.language.startsWith("zh");
  // An installed skill is listed from its own SKILL.md, which usually carries
  // English text only; the catalogue always has the zh pair, so lend it to the
  // installed row (desktop does the same). Rows read in the system language.
  const zhText = (skill: AvailableSkill | InstalledSkill) => {
    const catalogue = availableById.get(skill.id);
    return { name: skill.nameZh || catalogue?.nameZh, description: skill.descriptionZh || catalogue?.descriptionZh };
  };
  const label = (skill: AvailableSkill | InstalledSkill) => (chinese && zhText(skill).name) || skill.name || skill.id;
  const description = (skill: AvailableSkill | InstalledSkill) => (chinese && zhText(skill).description) || skill.description;
  const needle = query.trim().toLowerCase();
  const rows = (tab === "installed" ? installed.data ?? [] : available.data ?? []).filter(skill =>
    `${skill.id} ${label(skill)} ${description(skill)}`.toLowerCase().includes(needle));

  const mutate = async (operation: () => Promise<unknown>) => {
    if (disabled || writing.current || !active.current) return;
    writing.current = true;
    setBusy(true);
    setFailed(false);
    setConfirmRemove(null);
    try {
      await operation();
    } catch {
      if (active.current) setFailed(true);
    } finally {
      writing.current = false;
      if (active.current) {
        await installed.reload();
        if (active.current) setBusy(false);
      }
    }
  };
  const upgradeAll = () => mutate(async () => {
    for (const skill of upgrades) {
      // Closing this page/switching desktops stops the remaining batch. Never
      // let a new connection inherit operations authorized for the old one.
      if (!active.current) break;
      await remote.installSkill(skill.id, skill.latestVersion!);
    }
  });

  return <FlatList<AvailableSkill | InstalledSkill>
    data={rows} keyExtractor={skill => skill.id} contentContainerStyle={settingsStyles.content}
    keyboardShouldPersistTaps="handled"
    ListHeaderComponent={<View style={settingsStyles.section}>
      {/* A skill's own lifecycle: upgrading it without being asked, and having
          one recommended. The desktop keeps these two on its skills page, so
          they are named here the same way rather than sitting under automation. */}
      <SettingsSwitch label={t("desktopSettings.autoUpgradeSkills")} description={t("desktopSettings.autoUpgradeSkillsHint")}
        value={settings?.autoUpgradeSkills ?? false} disabled={settingsDisabled}
        onChange={autoUpgradeSkills => onChange?.({ autoUpgradeSkills })} />
      <SettingsSwitch label={t("desktopSettings.skillRecommend")} description={t("desktopSettings.skillRecommendHint")}
        value={settings?.skillRecommend ?? false} disabled={settingsDisabled}
        onChange={skillRecommend => onChange?.({ skillRecommend })} />
      <Text style={settingsStyles.description}>{t("desktopSettings.skillsHint")}</Text>
      <View style={settingsStyles.actions}>
        <Button compact label={t("desktopSettings.installed")} onPress={() => setTab("installed")} variant={tab === "installed" ? "primary" : "secondary"} />
        <Button compact label={t("desktopSettings.available")} onPress={() => setTab("available")} variant={tab === "available" ? "primary" : "secondary"} />
      </View>
      <TextInput accessibilityLabel={t("desktopSettings.searchSkills")} placeholder={t("desktopSettings.searchSkills")}
        placeholderTextColor={colors.inkMuted} value={query} onChangeText={setQuery} style={settingsStyles.search} />
      <ResourceStatus loading={installed.loading || available.loading} failed={installed.failed || available.failed}
        onReload={() => { void installed.reload(); void available.reload(); }} />
      {failed ? <Text accessibilityRole="alert" style={settingsStyles.error}>{t("desktopSettings.skillFailed")}</Text> : null}
      <Button compact label={t("desktopSettings.upgradeAll", { count: upgrades.length })} loading={busy}
        disabled={disabled || available.failed || available.loading || !upgrades.length} onPress={() => void upgradeAll()} variant="secondary" />
    </View>}
    renderItem={({ item }) => {
      const current = installedById.get(item.id);
      const catalog = availableById.get(item.id);
      const upgrade = isSkillUpgrade(current?.version, catalog?.latestVersion);
      return <View style={[settingsStyles.section, settingsStyles.card]}>
        <Text style={settingsStyles.label}>{label(item)}</Text>
        <Text numberOfLines={3} style={settingsStyles.description}>{description(item)}</Text>
        <Text style={settingsStyles.description}>{item.id}{current ? ` · ${current.version || t("desktopSettings.unversioned")}` : ""}
          {catalog?.latestVersion ? ` · ${t("desktopSettings.latest", { version: catalog.latestVersion })}` : ""}</Text>
        <View style={settingsStyles.actions}>
          {(!current || upgrade) && catalog?.latestVersion ? <Button compact
            label={t(current ? "desktopSettings.upgrade" : "desktopSettings.install")} disabled={disabled || available.failed || available.loading}
            onPress={() => void mutate(() => remote.installSkill(item.id, catalog.latestVersion!))} /> : null}
          {current ? confirmRemove === item.id ? <>
            <Text style={settingsStyles.description}>{t("desktopSettings.confirmRemove", { name: label(item) })}</Text>
            <Button compact label={t("desktopSettings.remove")} variant="danger" disabled={disabled}
              onPress={() => void mutate(async () => {
                const removed = await remote.uninstallSkill(item.id);
                if (!removed) throw new Error("skill_not_removed");
              })} />
            <Button compact label={t("chat.cancel")} variant="secondary" disabled={busy} onPress={() => setConfirmRemove(null)} />
          </> : <Button compact label={t("desktopSettings.uninstall")} variant="secondary" disabled={disabled} onPress={() => setConfirmRemove(item.id)} /> : null}
        </View>
      </View>;
    }}
    ListEmptyComponent={!(installed.loading || available.loading) ? <Text style={settingsStyles.description}>{t("desktopSettings.noSkills")}</Text> : undefined}
  />;
}
