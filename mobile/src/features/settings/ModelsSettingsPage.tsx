import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react-native";
import { Pressable, SectionList, StyleSheet, Switch, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { useRemoteControls } from "../../remote/RemoteContext";
import { modelReference, type DesktopSettings, type RemoteModel } from "../../remote/types";
import { colors, spacing } from "../../theme/tokens";
import { ResourceStatus, SettingsSwitch, settingsStyles } from "./SettingsPrimitives";
import { useDesktopResource } from "./useDesktopResource";

/** One provider and the models it contributes to the visibility list. */
interface ModelGroup {
  id: string;
  title: string;
  models: RemoteModel[];
  /** The models actually rendered: a folded group renders none. */
  data: RemoteModel[];
  expanded: boolean;
}

/**
 * One provider heading. Tapping the title folds the group; the switch is the
 * same bulk toggle the flat list used to offer, and stays outside the press
 * target so flipping it never folds the group.
 */
function GroupHeader({ description, disabled, expanded, foldDisabled, title, value, onChange, onFold }: {
  description: string;
  disabled: boolean;
  expanded: boolean;
  foldDisabled: boolean;
  title: string;
  value: boolean;
  onChange(visible: boolean): void;
  onFold(): void;
}) {
  const { t } = useTranslation();
  return <View style={[settingsStyles.row, styles.group]}>
    <Pressable accessibilityLabel={title} accessibilityRole="button"
      accessibilityState={{ disabled: foldDisabled, expanded }} disabled={foldDisabled} onPress={onFold}
      style={styles.groupLabel}>
      {expanded ? <ChevronDown size={18} color={colors.inkMuted} /> : <ChevronRight size={18} color={colors.inkMuted} />}
      <View style={styles.groupText}>
        <Text style={styles.groupTitle}>{title}</Text>
        <Text style={settingsStyles.description}>{description}</Text>
      </View>
    </Pressable>
    <Switch accessibilityLabel={t("desktopSettings.toggleProvider", { name: title })} disabled={disabled}
      onValueChange={onChange} trackColor={{ false: colors.line, true: colors.accent }} value={value} />
  </View>;
}

export function ModelsSettingsPage({ settings, disabled, onChange }: {
  settings: DesktopSettings | null;
  disabled: boolean;
  onChange(patch: Partial<DesktopSettings>): void;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const resource = useDesktopResource(remote.listSettingsModels, remote.desktopSettingsRevision, remote.desktopOnline);
  // Which providers can actually be called is desktop state too: one is either
  // signed in (the account provider — signing in writes its key) or holds a
  // saved API key. A desktop too old to report credentials keeps the full list.
  const providersSupported = remote.capabilities?.has("provider_management_v1") ?? false;
  const providers = useDesktopResource(remote.listProviders, remote.desktopSettingsRevision, remote.desktopOnline && providersSupported);
  const [query, setQuery] = useState("");
  const [folded, setFolded] = useState<ReadonlySet<string>>(() => new Set());
  const hidden = useMemo(() => new Set(settings?.hiddenModels ?? []), [settings?.hiddenModels]);

  const needle = query.trim().toLowerCase();
  const searching = needle.length > 0;
  const providerNames = useMemo(() => new Map(
    [...providers.data?.builtin ?? [], ...providers.data?.custom ?? []].map(provider => [provider.id, provider.name]),
  ), [providers.data]);
  const callable = useMemo(() => {
    const view = providers.data;
    if (!view) return null;
    return new Set([...view.builtin, ...view.custom].filter(provider => provider.hasApiKey).map(provider => provider.id));
  }, [providers.data]);

  const groups = useMemo<ModelGroup[]>(() => {
    const byProvider = new Map<string, RemoteModel[]>();
    for (const model of resource.data ?? []) {
      const provider = model.provider ?? "";
      // A provider with no credential cannot be called, so its models are not
      // offered here. Models the desktop reports without a provider are kept —
      // there is nothing to judge them by.
      if (provider && callable && !callable.has(provider)) continue;
      const name = providerNames.get(provider) ?? provider;
      if (needle && !`${provider} ${name} ${model.label ?? ""} ${model.id}`.toLowerCase().includes(needle)) continue;
      const models = byProvider.get(provider) ?? [];
      models.push(model);
      byProvider.set(provider, models);
    }
    return [...byProvider].map(([id, models]) => {
      const expanded = searching || !folded.has(id);
      // A folded group keeps its heading (and bulk switch) but no model rows.
      return { id, title: providerNames.get(id) ?? id, models, data: expanded ? models : [], expanded };
    }).sort((left, right) => left.title.localeCompare(right.title));
  }, [callable, folded, needle, providerNames, resource.data, searching]);

  const setVisibility = (models: RemoteModel[], visible: boolean) => {
    if (!settings || disabled) return;
    const next = new Set(settings.hiddenModels);
    for (const model of models) {
      if (visible) next.delete(modelReference(model));
      else next.add(modelReference(model));
    }
    onChange({ hiddenModels: [...next] });
  };
  const toggleFolded = (id: string) => setFolded(current => {
    const next = new Set(current);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  });

  const rowsDisabled = disabled || !settings || resource.loading || resource.failed;
  return <SectionList<RemoteModel, ModelGroup>
    sections={groups}
    keyExtractor={modelReference}
    contentContainerStyle={settingsStyles.content}
    keyboardShouldPersistTaps="handled"
    stickySectionHeadersEnabled={false}
    ListHeaderComponent={<View style={settingsStyles.section}>
      <Text style={settingsStyles.description}>{t("desktopSettings.modelsHint")}</Text>
      <TextInput accessibilityLabel={t("desktopSettings.searchModels")} placeholder={t("desktopSettings.searchModels")}
        placeholderTextColor={colors.inkMuted} value={query} onChangeText={setQuery} style={settingsStyles.search} />
      <ResourceStatus loading={resource.loading || providers.loading}
        failed={resource.failed || (providersSupported && providers.failed)}
        onReload={() => { void resource.reload(); void providers.reload(); }} />
    </View>}
    renderSectionHeader={({ section }) => {
      const hiddenCount = section.models.filter(model => hidden.has(modelReference(model))).length;
      const description = hiddenCount > 0
        ? `${t("desktopSettings.providerModelCount", { count: section.models.length })} · ${t("desktopSettings.hiddenModelCount", { count: hiddenCount })}`
        : t("desktopSettings.providerModelCount", { count: section.models.length });
      return <GroupHeader description={description} disabled={rowsDisabled} expanded={section.expanded}
        foldDisabled={searching} onFold={() => toggleFolded(section.id)}
        onChange={visible => setVisibility(section.models, visible)}
        title={section.title || t("desktopSettings.models")}
        value={section.models.every(model => !hidden.has(modelReference(model)))} />;
    }}
    renderItem={({ item }) => <View style={styles.model}>
      <SettingsSwitch label={item.label || item.id} description={item.id}
        value={!hidden.has(modelReference(item))} disabled={rowsDisabled}
        onChange={visible => setVisibility([item], visible)} />
    </View>}
    ListEmptyComponent={!resource.loading
      ? <Text style={settingsStyles.description}>
        {t(searching ? "desktopSettings.noModels" : "desktopSettings.noUsableProviders")}
      </Text>
      : null}
  />;
}

const styles = StyleSheet.create({
  group: { backgroundColor: colors.surfaceSubtle },
  groupLabel: { alignItems: "center", flex: 1, flexDirection: "row", gap: spacing.sm, minWidth: 0 },
  groupText: { flex: 1, minWidth: 0, gap: spacing.xs },
  groupTitle: { color: colors.ink, flexShrink: 1, fontSize: 15, fontWeight: "600" },
  model: { marginStart: spacing.lg },
});
