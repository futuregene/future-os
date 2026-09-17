import { useEffect, useRef, useState } from "react";
import { FlatList, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/Button";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { AvailableSkill, InstalledSkill } from "../../remote/types";
import { colors } from "../../theme/tokens";
import { ResourceStatus, settingsStyles } from "./SettingsPrimitives";
import { isSkillUpgrade } from "./skillVersion";
import { useDesktopResource } from "./useDesktopResource";

export function SkillsSettingsPage() {
  const { t, i18n } = useTranslation();
  const remote = useRemoteControls();
  const installed = useDesktopResource(remote.listInstalledSkills, remote.skillsRevision, remote.desktopOnline);
  const available = useDesktopResource(remote.listAvailableSkills, 0, remote.desktopOnline);
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
  const label = (skill: AvailableSkill | InstalledSkill) => (chinese && skill.nameZh) || skill.name || skill.id;
  const description = (skill: AvailableSkill | InstalledSkill) => (chinese && skill.descriptionZh) || skill.description;
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
      return <View style={settingsStyles.section}>
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
    ListEmptyComponent={!(installed.loading || available.loading) ? <Text style={settingsStyles.description}>{t("desktopSettings.noSkills")}</Text> : null}
  />;
}
