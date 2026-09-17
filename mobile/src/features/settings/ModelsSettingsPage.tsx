import { useMemo, useState } from "react";
import { SectionList, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { useRemoteControls } from "../../remote/RemoteContext";
import { modelReference, type DesktopSettings, type RemoteModel } from "../../remote/types";
import { colors } from "../../theme/tokens";
import { ResourceStatus, SettingsSwitch, settingsStyles } from "./SettingsPrimitives";
import { useDesktopResource } from "./useDesktopResource";

export function ModelsSettingsPage({ settings, disabled, onChange }: {
  settings: DesktopSettings | null;
  disabled: boolean;
  onChange(patch: Partial<DesktopSettings>): void;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const resource = useDesktopResource(remote.listSettingsModels, remote.desktopSettingsRevision, remote.desktopOnline);
  const [query, setQuery] = useState("");
  const hidden = new Set(settings?.hiddenModels ?? []);
  const sections = useMemo(() => {
    const groups = new Map<string, RemoteModel[]>();
    const needle = query.trim().toLowerCase();
    for (const model of resource.data ?? []) {
      if (!`${model.provider ?? ""} ${model.label ?? ""} ${model.id}`.toLowerCase().includes(needle)) continue;
      const provider = model.provider ?? "";
      const list = groups.get(provider) ?? [];
      list.push(model);
      groups.set(provider, list);
    }
    return [...groups].sort(([a], [b]) => a.localeCompare(b)).map(([title, data]) => ({ title, data }));
  }, [query, resource.data]);

  const setVisibility = (models: RemoteModel[], visible: boolean) => {
    if (!settings || disabled) return;
    const next = new Set(settings.hiddenModels);
    for (const model of models) {
      if (visible) next.delete(modelReference(model));
      else next.add(modelReference(model));
    }
    onChange({ hiddenModels: [...next] });
  };

  return <SectionList
    sections={sections}
    keyExtractor={modelReference}
    contentContainerStyle={settingsStyles.content}
    keyboardShouldPersistTaps="handled"
    stickySectionHeadersEnabled={false}
    ListHeaderComponent={<View style={settingsStyles.section}>
      <Text style={settingsStyles.description}>{t("desktopSettings.modelsHint")}</Text>
      <TextInput accessibilityLabel={t("desktopSettings.searchModels")} placeholder={t("desktopSettings.searchModels")}
        placeholderTextColor={colors.inkMuted} value={query} onChangeText={setQuery} style={settingsStyles.search} />
      <ResourceStatus loading={resource.loading} failed={resource.failed} onReload={() => void resource.reload()} />
    </View>}
    renderSectionHeader={({ section }) => <SettingsSwitch label={section.title || t("desktopSettings.models")}
      description={t("desktopSettings.toggleGroup")} value={section.data.every(model => !hidden.has(modelReference(model)))}
      disabled={disabled || !settings || resource.loading || resource.failed}
      onChange={visible => setVisibility(section.data, visible)} />}
    renderItem={({ item }) => <SettingsSwitch label={item.label || item.id} description={item.id}
      value={!hidden.has(modelReference(item))} disabled={disabled || !settings || resource.loading || resource.failed}
      onChange={visible => setVisibility([item], visible)} />}
    ListEmptyComponent={!resource.loading ? <Text style={settingsStyles.description}>{t("desktopSettings.noModels")}</Text> : null}
  />;
}
