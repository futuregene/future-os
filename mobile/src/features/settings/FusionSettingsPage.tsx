import { useState } from "react";
import type { FusionConfiguration, FusionModelConfig, FusionSettings } from "@future-os/thread-projection";
import { fusionSettingsIssue, newFusionParticipant } from "@future-os/thread-projection";
import { ChevronDown, Trash2, X } from "lucide-react-native";
import { FlatList, Keyboard, Modal, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { useTranslation } from "react-i18next";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { Button } from "../../components/Button";
import { thinkingLevelLabel } from "../../i18n/thinkingLevelLabel";
import { useRemoteControls } from "../../remote/RemoteContext";
import { modelReference, modelThinkingLevels, type RemoteModel } from "../../remote/types";
import { createThemedStyles, useTheme } from "../../theme/ThemeContext";
import { layout, radius, spacing } from "../../theme/tokens";
import { ResourceStatus, SettingsSection, SettingsSwitch, useSettingsStyles } from "./SettingsPrimitives";
import { useDesktopResource } from "./useDesktopResource";

function Editor({ snapshot, models, providerNames, reload }: { snapshot: FusionSettings; models: RemoteModel[]; providerNames: Record<string, string>; reload(): void }) {
  const remote = useRemoteControls();
  const { t } = useTranslation();
  const styles = useStyles();
  const settingsStyles = useSettingsStyles();
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const [saved, setSaved] = useState(snapshot);
  const [draft, setDraft] = useState(snapshot.configuration);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);
  const [picker, setPicker] = useState<{ index: number; field: "model" | "thinking" } | null>(null);
  const [query, setQuery] = useState("");
  const conflict = snapshot.revision > saved.revision;
  const locked = busy || !remote.desktopReadable || conflict;
  const issue = fusionSettingsIssue(draft);
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved.configuration);
  async function persist(configuration?: FusionConfiguration, enabled?: boolean) {
    if (locked) return;
    setBusy(true); setError(null); setDone(false);
    try {
      const next = await remote.updateFusionSettings({ expected_revision: saved.revision, configuration, enabled });
      setSaved(next);
      if (configuration) { setDraft(next.configuration); setDone(true); }
      reload();
    } catch (error) { setError(error instanceof Error ? error.message : String(error)); reload(); }
    finally { setBusy(false); }
  }
  const role = (index: number) => index < 0 ? t("desktopSettings.fusionModerator") : t("desktopSettings.fusionParticipant", { number: index + 1 });
  const profile = (index: number) => index < 0 ? draft.moderator : draft.panel[index];
  function change(index: number, value: FusionModelConfig) {
    setDone(false);
    setDraft(index < 0 ? { ...draft, moderator: value } : { ...draft, panel: draft.panel.map((entry, i) => i === index ? value : entry) });
  }
  function open(index: number, field: "model" | "thinking") { Keyboard.dismiss(); setQuery(""); setPicker({ index, field }); }
  const displayName = (model: RemoteModel) => `${providerNames[model.provider ?? ""] ?? model.provider ?? ""}/${model.label || model.id}`;
  function row(value: FusionModelConfig, index: number) {
    const selected = models.find(model => modelReference(model) === value.model);
    return <View key={index} style={styles.modelCard}>
      <View style={styles.heading}><Text style={settingsStyles.label}>{role(index)}</Text>{index >= 0 && draft.panel.length > 2 ? <Pressable accessibilityRole="button" accessibilityLabel={t("desktopSettings.fusionRemoveParticipant", { name: role(index) })} disabled={locked} style={styles.remove} onPress={() => setDraft({ ...draft, panel: draft.panel.filter((_, i) => i !== index) })}><Trash2 size={18} color={colors.inkMuted} /></Pressable> : null}</View>
      <Pressable accessibilityRole="button" accessibilityLabel={`${role(index)} · ${t("desktopSettings.fusionModel")}`} disabled={locked} style={styles.choice} onPress={() => open(index, "model")}><Text style={styles.choiceText}>{selected ? displayName(selected) : value.model || t("desktopSettings.fusionChooseModel")}</Text><ChevronDown size={16} color={colors.inkMuted} /></Pressable>
      <Pressable accessibilityRole="button" accessibilityLabel={`${role(index)} · ${t("desktopSettings.fusionThinking")}`} disabled={locked || !selected} style={styles.choice} onPress={() => open(index, "thinking")}><Text style={styles.choiceText}>{t("desktopSettings.fusionThinking")} · {thinkingLevelLabel(t, value.thinking_level)}</Text><ChevronDown size={16} color={colors.inkMuted} /></Pressable>
    </View>;
  }
  const picked = picker ? profile(picker.index) : null;
  const pickedModel = picked ? models.find(model => modelReference(model) === picked.model) : undefined;
  const choices = picker?.field === "thinking" ? modelThinkingLevels(pickedModel).map(level => ({ id: level, label: thinkingLevelLabel(t, level) })) : models.filter(model => `${displayName(model)} ${model.id} ${model.provider}`.toLowerCase().includes(query.toLowerCase())).map(model => ({ id: modelReference(model), label: displayName(model) }));
  return <>
    <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      <SettingsSwitch label={t("desktopSettings.fusionEnabled")} description={t("desktopSettings.fusionEnabledHint")} disabled={locked} value={saved.enabled} onChange={enabled => void persist(undefined, enabled)} />
      <Text style={settingsStyles.description}>{t("desktopSettings.fusionRolesHint")}</Text>
      {conflict ? <View><Text accessibilityRole="alert" style={styles.error}>{t("desktopSettings.fusionConflict")}</Text><Button label={t("desktopSettings.fusionReload")} variant="secondary" onPress={() => { setSaved(snapshot); setDraft(snapshot.configuration); setError(null); }} /></View> : null}
      {row(draft.moderator, -1)}
      <SettingsSection title={t("desktopSettings.fusionParticipants")}><Text style={settingsStyles.description}>{t("desktopSettings.fusionParticipantsHint")}</Text>{draft.panel.map(row)}</SettingsSection>
      <Button label={t("desktopSettings.fusionAddParticipant")} variant="secondary" disabled={locked || draft.panel.length >= 5} onPress={() => { setDone(false); setDraft({ ...draft, panel: [...draft.panel, newFusionParticipant()] }); }} />
      {issue ? <Text accessibilityRole="alert" style={styles.error}>{t(`desktopSettings.fusion${issue.charAt(0).toUpperCase()}${issue.slice(1)}`)}</Text> : null}
      {error ? <Text accessibilityRole="alert" style={styles.error}>{error}</Text> : null}
      <Button label={t("desktopSettings.fusionSave")} loading={busy} disabled={locked || !!issue || !dirty} onPress={() => void persist(draft)} />
      {done ? <Text accessibilityLiveRegion="polite" style={settingsStyles.description}>{t("desktopSettings.fusionSaved")}</Text> : null}
    </ScrollView>
    <Modal visible={picker !== null} transparent animationType="slide" onRequestClose={() => setPicker(null)}>
      <View style={[styles.overlay, { paddingTop: insets.top + layout.gutter, paddingBottom: insets.bottom + layout.gutter }]}>
        <Pressable accessible={false} style={StyleSheet.absoluteFill} onPress={() => setPicker(null)} />
        <View accessibilityViewIsModal style={styles.picker}>
          <View style={styles.heading}><Text accessibilityRole="header" style={settingsStyles.label}>{picker ? role(picker.index) : ""} · {t(picker?.field === "thinking" ? "desktopSettings.fusionThinking" : "desktopSettings.fusionModel")}</Text><Pressable accessibilityRole="button" accessibilityLabel={t("common.close")} style={styles.remove} onPress={() => setPicker(null)}><X size={20} color={colors.inkMuted} /></Pressable></View>
          {picker?.field === "model" ? <TextInput accessibilityLabel={t("desktopSettings.searchModels")} placeholder={t("desktopSettings.searchModels")} placeholderTextColor={colors.inkMuted} value={query} onChangeText={setQuery} style={settingsStyles.search} /> : null}
          <FlatList data={choices} keyExtractor={item => item.id} keyboardShouldPersistTaps="handled" renderItem={({ item }) => <Pressable accessibilityRole="radio" accessibilityState={{ checked: (picker?.field === "thinking" ? picked?.thinking_level : picked?.model) === item.id }} style={styles.option} onPress={() => {
            if (!picker || !picked || locked) return;
            const selected = models.find(model => modelReference(model) === item.id);
            const levels = modelThinkingLevels(selected);
            change(picker.index, picker.field === "thinking" ? { ...picked, thinking_level: item.id } : { ...newFusionParticipant(), model: item.id, thinking_level: levels.includes(picked.thinking_level) ? picked.thinking_level : levels[0] ?? "default" });
            setPicker(null);
          }}><Text style={styles.choiceText}>{item.label}</Text></Pressable>} />
        </View>
      </View>
    </Modal>
  </>;
}
export function FusionSettingsPage() {
  const remote = useRemoteControls();
  const enabled = remote.desktopReadable && remote.capabilities.has("fusion_settings_v1");
  const settings = useDesktopResource(remote.getFusionSettings, remote.desktopSettingsRevision, enabled);
  const models = useDesktopResource(remote.listSettingsModels, remote.desktopSettingsRevision, enabled);
  const providers = useDesktopResource(remote.listProviders, remote.desktopSettingsRevision, enabled);
  const providerNames = Object.fromEntries([...(providers.data?.builtin ?? []), ...(providers.data?.custom ?? [])].map(provider => [provider.id, provider.name]));
  return settings.data && models.data ? <Editor snapshot={settings.data} models={models.data} providerNames={providerNames} reload={() => { void settings.reload(); void models.reload(); }} /> : <View><ResourceStatus loading={settings.loading || models.loading} failed={settings.failed || models.failed} onReload={() => { void settings.reload(); void models.reload(); }} /></View>;
}
const useStyles = createThemedStyles(theme => StyleSheet.create({
  modelCard: { padding: spacing.md, gap: spacing.sm, borderRadius: radius.lg, borderWidth: 1, borderColor: theme.colors.line },
  heading: { flexDirection: "row", justifyContent: "space-between", alignItems: "center", gap: spacing.sm },
  choice: { minHeight: layout.touchTarget, flexDirection: "row", alignItems: "center", gap: spacing.sm, paddingHorizontal: spacing.sm, borderRadius: radius.sm, borderWidth: 1, borderColor: theme.colors.line },
  choiceText: { flex: 1, color: theme.colors.ink, fontSize: theme.font(14) },
  remove: { width: layout.touchTarget, height: layout.touchTarget, alignItems: "center", justifyContent: "center" },
  error: { color: theme.colors.danger, fontSize: theme.font(13) },
  overlay: { flex: 1, justifyContent: "center", paddingHorizontal: layout.gutter, backgroundColor: theme.colors.overlay },
  picker: { maxHeight: "80%", padding: spacing.md, gap: spacing.sm, borderRadius: radius.lg, backgroundColor: theme.colors.surface },
  option: { minHeight: layout.touchTarget, justifyContent: "center", padding: spacing.md },
}));
