import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Pressable, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { ChevronDown, ChevronRight, Trash2 } from "lucide-react-native";
import { ActionMenu } from "../../components/ActionMenu";
import { Button } from "../../components/Button";
import { thinkingLevelLabel } from "../../i18n/thinkingLevelLabel";
import { useRemoteControls } from "../../remote/RemoteContext";
import {
  CUSTOM_PROVIDER_APIS,
  DEFAULT_THINKING_LEVEL,
  type CustomProviderUpsert,
  type RemoteCustomProvider,
  type RemoteProviderModel,
} from "../../remote/types";
import { radius, spacing } from "../../theme/tokens";
import { createThemedStyles, useTheme } from "../../theme/ThemeContext";
import { SettingsField, SettingsSwitch, useSettingsStyles } from "./SettingsPrimitives";

/** Mirrors the desktop form defaults for a freshly added model. */
const DEFAULT_CONTEXT_WINDOW = "128000";
const DEFAULT_MAX_TOKENS = "16384";
/**
 * The thinking levels the editor offers per model. `"default"` — send no
 * thinking parameter — is always accepted and so is never a toggle; every other
 * entry is a wire value the model receives verbatim.
 */
const THINKING_LEVEL_OPTIONS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const;
/** A new model starts on the levels the desktop's own form seeds. */
const NEW_MODEL_THINKING_LEVELS = ["off", "low", "medium", "high"];
/** Mirrors the host's cap on one model's declared list. */
const MAX_THINKING_LEVELS = 16;
const API_LABELS: Record<string, string> = {
  "openai-completions": "OpenAI Completions",
  "openai-responses": "OpenAI Responses",
  "anthropic": "Anthropic",
};

type PriceField = "inputCost" | "outputCost" | "cacheReadCost" | "cacheWriteCost";
const PRICE_FIELDS: [PriceField, string][] = [
  ["inputCost", "desktopSettings.modelPriceInput"],
  ["outputCost", "desktopSettings.modelPriceOutput"],
  ["cacheReadCost", "desktopSettings.modelPriceCacheRead"],
  ["cacheWriteCost", "desktopSettings.modelPriceCacheWrite"],
];

/**
 * An editable model row. `key` is a stable identity for reconciliation — the
 * model id itself is user-editable and may start empty. Numbers stay strings so
 * a trailing decimal point survives typing (the desktop form does the same).
 */
interface ModelDraft {
  key: string;
  id: string;
  name: string;
  supportsImages: boolean;
  /**
   * Kept for the wire only — an older desktop still reads it, and the phone no
   * longer shows a switch for it. The levels below decide what is offered.
   */
  reasoning: boolean;
  /** The declared levels, `"default"` implicit and never stored. */
  thinkingLevels: string[];
  contextWindow: string;
  maxTokens: string;
  inputCost: string;
  outputCost: string;
  cacheReadCost: string;
  cacheWriteCost: string;
}

/**
 * A stored level list in the shape the editor's toggles use: `"default"` is
 * implicit (never stored), entries are trimmed, unique and non-empty. Unknown
 * tokens survive — the wire accepts any value, so an edit must not silently drop
 * a level someone hand-wrote into `models.json`.
 */
function editableThinkingLevels(levels: string[] | undefined): string[] {
  const result: string[] = [];
  for (const raw of levels ?? []) {
    const level = raw.trim();
    if (!level || level === DEFAULT_THINKING_LEVEL || result.includes(level)) continue;
    result.push(level);
  }
  return result;
}

/** The toggles one model shows: the offer above, then any stored extra token. */
function thinkingLevelOptions(levels: string[]): string[] {
  return [
    ...THINKING_LEVEL_OPTIONS,
    ...levels.filter(level => !(THINKING_LEVEL_OPTIONS as readonly string[]).includes(level)),
  ];
}

function toDraft(model: RemoteProviderModel, key: string): ModelDraft {
  const price = (value?: number) => (value ? String(value) : "");
  return {
    key,
    id: model.id,
    name: model.name ?? "",
    supportsImages: model.supportsImages ?? false,
    reasoning: model.reasoning ?? true,
    thinkingLevels: editableThinkingLevels(model.thinkingLevels),
    contextWindow: String(model.contextWindow ?? Number(DEFAULT_CONTEXT_WINDOW)),
    maxTokens: String(model.maxTokens ?? Number(DEFAULT_MAX_TOKENS)),
    inputCost: price(model.inputCost),
    outputCost: price(model.outputCost),
    cacheReadCost: price(model.cacheReadCost),
    cacheWriteCost: price(model.cacheWriteCost),
  };
}

function newDraft(key: string): ModelDraft {
  return {
    key,
    id: "",
    name: "",
    supportsImages: false,
    reasoning: true,
    thinkingLevels: [...NEW_MODEL_THINKING_LEVELS],
    contextWindow: DEFAULT_CONTEXT_WINDOW,
    maxTokens: DEFAULT_MAX_TOKENS,
    inputCost: "",
    outputCost: "",
    cacheReadCost: "",
    cacheWriteCost: "",
  };
}

/** Desktop-authored validation text is shown as-is, like in the desktop form. */
function detail(failure: unknown): string {
  return failure instanceof Error ? failure.message : String(failure);
}

/** The desktop accepts only a parseable http(s) address; mirror that check. */
function isHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  }
  catch {
    return false;
  }
}

/**
 * One custom provider with its models. The whole provider — including every
 * model's limits and prices — is submitted as a single desktop write, so the
 * desktop and the phone can never disagree about a half-saved provider.
 */
export function CustomProviderPage({ provider, onDone }: {
  provider: RemoteCustomProvider | null;
  /** Leave the page once the desktop confirmed the write (save or delete). */
  onDone(): void;
}) {
  const settingsStyles = useSettingsStyles();
  const styles = useStyles();
  const { colors } = useTheme();
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const editing = provider !== null;
  const [id, setId] = useState(provider?.id ?? "");
  const [name, setName] = useState(provider?.name ?? "");
  const [api, setApi] = useState(provider?.api || CUSTOM_PROVIDER_APIS[0]);
  const [baseUrl, setBaseUrl] = useState(provider?.baseUrl ?? "");
  // Unlike an edit, a new provider's key is written only when one is typed.
  const [apiKey, setApiKey] = useState("");
  const [models, setModels] = useState<ModelDraft[]>(
    () => provider?.models.map((model, index) => toDraft(model, `m${index}`)) ?? [],
  );
  const [expanded, setExpanded] = useState<string | null>(null);
  const [pickingApi, setPickingApi] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  const patchModel = (key: string, patch: Partial<ModelDraft>) =>
    setModels(current => current.map(model => (model.key === key ? { ...model, ...patch } : model)));

  const addModel = () => {
    const draft = newDraft(`n${Date.now()}${models.length}`);
    setModels(current => [...current, draft]);
    setExpanded(draft.key);
  };

  const removeModel = (key: string) =>
    setModels(current => current.filter(model => model.key !== key));

  const setPrice = (key: string, field: PriceField, value: string) =>
    setModels(current => current.map(model => (model.key === key ? { ...model, [field]: value } : model)));

  const toggleThinkingLevel = (key: string, level: string) =>
    setModels(current => current.map(model => (model.key !== key ? model : {
      ...model,
      thinkingLevels: model.thinkingLevels.includes(level)
        ? model.thinkingLevels.filter(entry => entry !== level)
        : [...model.thinkingLevels, level],
    })));

  /** Local rules mirroring `agent_providers::validate`; the desktop re-checks. */
  function validate(): CustomProviderUpsert | string {
    const trimmedId = id.trim().toLowerCase();
    const trimmedUrl = baseUrl.trim();
    if (!editing) {
      if (!trimmedId)
        return t("desktopSettings.providerIdRequired");
      if (trimmedId.length < 2 || trimmedId.length > 40)
        return t("desktopSettings.providerIdLength");
      if (!/^[a-z0-9_-]+$/.test(trimmedId))
        return t("desktopSettings.providerIdPattern");
    }
    if (!isHttpUrl(trimmedUrl))
      return t("desktopSettings.baseUrlInvalid");
    const seen = new Set<string>();
    const cleaned: RemoteProviderModel[] = [];
    for (const model of models) {
      const modelId = model.id.trim();
      if (!modelId)
        return t("desktopSettings.modelIdRequired");
      if (seen.has(modelId))
        return t("desktopSettings.modelIdDuplicate", { id: modelId });
      seen.add(modelId);
      const contextWindow = Number(model.contextWindow);
      const maxTokens = Number(model.maxTokens);
      if (!Number.isSafeInteger(contextWindow) || contextWindow <= 0
        || !Number.isSafeInteger(maxTokens) || maxTokens <= 0)
        return t("desktopSettings.modelLimitsInvalid", { id: modelId });
      if (maxTokens > contextWindow)
        return t("desktopSettings.modelMaxTokensExceedsContext", { id: modelId });
      const prices: Record<PriceField, number> = { inputCost: 0, outputCost: 0, cacheReadCost: 0, cacheWriteCost: 0 };
      for (const [field, raw] of PRICE_FIELDS.map(([field]) => [field, model[field]] as const)) {
        const price = raw.trim() === "" ? 0 : Number(raw);
        if (!Number.isFinite(price) || price < 0)
          return t("desktopSettings.modelPriceInvalid", { id: modelId });
        prices[field] = price;
      }
      // Mirrors the host: trim, drop empties/duplicates and the implicit
      // `default`, and refuse a list past the cap. `reasoning` rides along
      // untouched — it is no longer a control here but an older host reads it.
      const thinkingLevels = editableThinkingLevels(model.thinkingLevels);
      if (thinkingLevels.length > MAX_THINKING_LEVELS)
        return t("desktopSettings.modelThinkingLevelsMax", { id: modelId });
      cleaned.push({
        id: modelId,
        name: model.name.trim(),
        supportsImages: model.supportsImages,
        reasoning: model.reasoning,
        thinkingLevels,
        contextWindow,
        maxTokens,
        inputCost: prices.inputCost,
        outputCost: prices.outputCost,
        cacheReadCost: prices.cacheReadCost,
        cacheWriteCost: prices.cacheWriteCost,
      });
    }
    return {
      id: trimmedId || (provider?.id ?? ""),
      name: name.trim(),
      api,
      baseUrl: trimmedUrl,
      // Absent keeps the stored key; the desktop writes it only when non-empty.
      apiKey: apiKey.trim() ? apiKey.trim() : null,
      models: cleaned,
      create: !editing,
    };
  }

  const save = async () => {
    if (busy) return;
    const payload = validate();
    if (typeof payload === "string") {
      setError(payload);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await remote.upsertCustomProvider(payload);
      onDone();
    }
    catch (failure) {
      setError(detail(failure));
    }
    finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!provider || busy) return;
    setBusy(true);
    setError(null);
    try {
      await remote.deleteCustomProvider(provider.id);
      onDone();
    }
    catch (failure) {
      setError(detail(failure));
    }
    finally {
      setBusy(false);
    }
  };

  return <>
    <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
      <View style={[settingsStyles.card, styles.stack]}>
        <SettingsField
          hint={editing ? t("desktopSettings.providerIdHintLocked") : t("desktopSettings.providerIdHint")}
          label={t("desktopSettings.providerId")}
        >
          <TextInput accessibilityLabel={t("desktopSettings.providerId")} autoCapitalize="none" autoCorrect={false}
            editable={!editing} onChangeText={text => setId(text.toLowerCase())} placeholder="acme"
            placeholderTextColor={colors.inkMuted}
            style={[settingsStyles.input, editing && settingsStyles.inputDisabled]} value={id} />
        </SettingsField>
        <SettingsField label={t("desktopSettings.providerName")}>
          <TextInput accessibilityLabel={t("desktopSettings.providerName")} onChangeText={setName}
            placeholder={id || "Acme"} placeholderTextColor={colors.inkMuted} style={settingsStyles.input} value={name} />
        </SettingsField>
        <SettingsField label={t("desktopSettings.apiType")}>
          <Pressable accessibilityLabel={t("desktopSettings.apiType")} accessibilityRole="button"
            onPress={() => setPickingApi(true)} style={({ pressed }) => [settingsStyles.row, pressed && settingsStyles.pressed]}>
            <Text style={settingsStyles.label}>{API_LABELS[api] ?? api}</Text>
            <ChevronRight size={18} color={colors.inkMuted} />
          </Pressable>
        </SettingsField>
        <SettingsField label={t("desktopSettings.baseUrl")}>
          <TextInput accessibilityLabel={t("desktopSettings.baseUrl")} autoCapitalize="none" autoCorrect={false}
            keyboardType="url" onChangeText={setBaseUrl} placeholder="https://api.example.com/v1"
            placeholderTextColor={colors.inkMuted} style={settingsStyles.input} value={baseUrl} />
        </SettingsField>
        <SettingsField
          hint={editing ? t("desktopSettings.apiKeyKeepHint") : t("desktopSettings.apiKeyHint")}
          label={t("desktopSettings.apiKey")}
        >
          <TextInput accessibilityLabel={t("desktopSettings.apiKey")} autoCapitalize="none" autoCorrect={false}
            onChangeText={setApiKey} placeholder={provider?.hasApiKey ? t("desktopSettings.apiKeyKeep") : t("desktopSettings.apiKeyPlaceholder")}
            placeholderTextColor={colors.inkMuted} secureTextEntry style={settingsStyles.input} value={apiKey} />
        </SettingsField>
        {error ? <Text accessibilityRole="alert" style={settingsStyles.error}>{error}</Text> : null}
        <View style={settingsStyles.actions}>
          <Button disabled={busy} label={t("desktopSettings.save")} loading={busy} onPress={() => void save()} variant="primary" />
        </View>
      </View>

      <View style={[settingsStyles.card, styles.stack]}>
        <View style={styles.modelsHeader}>
          <Text style={settingsStyles.fieldLabel}>{`${t("desktopSettings.providerModels")} (${models.length})`}</Text>
          <Pressable accessibilityLabel={t("desktopSettings.addModel")} accessibilityRole="button" onPress={addModel}
            style={({ pressed }) => [styles.addModel, pressed && settingsStyles.pressed]}>
            <Text style={styles.addModelLabel}>{t("desktopSettings.addModel")}</Text>
          </Pressable>
        </View>
        {models.length === 0
          ? <Text style={settingsStyles.description}>{t("desktopSettings.noModels")}</Text>
          : models.map(model => {
            const open = expanded === model.key;
            const title = model.name.trim() || model.id.trim() || t("desktopSettings.newModel");
            const summary = [
              model.id.trim() || t("desktopSettings.modelIdMissing"),
              `${model.contextWindow}/${model.maxTokens}`,
              model.inputCost.trim() || model.outputCost.trim()
                ? `${t("desktopSettings.modelPriceInput")} ${model.inputCost || 0} · ${t("desktopSettings.modelPriceOutput")} ${model.outputCost || 0}`
                : t("desktopSettings.modelUnpriced"),
            ].join(" · ");
            return <View key={model.key} style={styles.model}>
              <Pressable accessibilityLabel={title} accessibilityRole="button"
                accessibilityState={{ expanded: open }}
                onPress={() => setExpanded(open ? null : model.key)}
                style={({ pressed }) => [styles.modelHeader, pressed && settingsStyles.pressed]}>
                <View style={settingsStyles.labelContainer}>
                  <Text style={settingsStyles.label}>{title}</Text>
                  <Text style={settingsStyles.description}>{summary}</Text>
                </View>
                {open ? <ChevronDown size={18} color={colors.inkMuted} /> : <ChevronRight size={18} color={colors.inkMuted} />}
              </Pressable>
              {open
                ? <View style={[styles.stack, styles.modelBody]}>
                  <SettingsField label={t("desktopSettings.modelId")}>
                    <TextInput accessibilityLabel={t("desktopSettings.modelId")} autoCapitalize="none" autoCorrect={false}
                      onChangeText={text => patchModel(model.key, { id: text })} placeholder="gpt-4o"
                      placeholderTextColor={colors.inkMuted} style={settingsStyles.input} value={model.id} />
                  </SettingsField>
                  <SettingsField label={t("desktopSettings.modelName")}>
                    <TextInput accessibilityLabel={t("desktopSettings.modelName")} onChangeText={text => patchModel(model.key, { name: text })}
                      placeholderTextColor={colors.inkMuted} style={settingsStyles.input} value={model.name} />
                  </SettingsField>
                  <SettingsField label={t("desktopSettings.modelContextWindow")}>
                    <TextInput accessibilityLabel={t("desktopSettings.modelContextWindow")} keyboardType="number-pad"
                      onChangeText={text => patchModel(model.key, { contextWindow: text })}
                      placeholderTextColor={colors.inkMuted} style={settingsStyles.input} value={model.contextWindow} />
                  </SettingsField>
                  <SettingsField label={t("desktopSettings.modelMaxTokens")}>
                    <TextInput accessibilityLabel={t("desktopSettings.modelMaxTokens")} keyboardType="number-pad"
                      onChangeText={text => patchModel(model.key, { maxTokens: text })}
                      placeholderTextColor={colors.inkMuted} style={settingsStyles.input} value={model.maxTokens} />
                  </SettingsField>
                  <SettingsSwitch label={t("desktopSettings.modelSupportsImages")} description={t("desktopSettings.modelSupportsImagesHint")}
                    value={model.supportsImages} onChange={value => patchModel(model.key, { supportsImages: value })} />
                  {/* Not a single "thinking" switch: the host declares the exact
                      levels a model accepts, so the phone offers them as a
                      multi-select. "Default" is implicit and always available. */}
                  <SettingsField hint={t("desktopSettings.modelThinkingLevelsHint")}
                    label={t("desktopSettings.modelThinkingLevels")}>
                    <View style={settingsStyles.actions}>
                      {thinkingLevelOptions(model.thinkingLevels).map(level => {
                        const selected = model.thinkingLevels.includes(level);
                        const label = thinkingLevelLabel(t, level);
                        return (
                          <Pressable accessibilityLabel={label} accessibilityRole="checkbox"
                            accessibilityState={{ checked: selected }} key={level}
                            onPress={() => toggleThinkingLevel(model.key, level)}
                            style={({ pressed }) => [styles.levelChip, selected && styles.levelChipSelected,
                              pressed && settingsStyles.pressed]}>
                            <Text style={[styles.levelChipLabel, selected && styles.levelChipLabelSelected]}>{label}</Text>
                          </Pressable>
                        );
                      })}
                    </View>
                  </SettingsField>
                  <Text style={settingsStyles.fieldLabel}>{t("desktopSettings.modelPrices")}</Text>
                  <View style={styles.priceGrid}>
                    {PRICE_FIELDS.map(([field, labelKey]) => <View key={field} style={styles.priceCell}>
                      <SettingsField label={t(labelKey)}>
                        <TextInput accessibilityLabel={t(labelKey)} keyboardType="decimal-pad"
                          onChangeText={text => setPrice(model.key, field, text)}
                          placeholder="0" placeholderTextColor={colors.inkMuted} style={settingsStyles.input} value={model[field]} />
                      </SettingsField>
                    </View>)}
                  </View>
                  <Text style={settingsStyles.description}>{t("desktopSettings.modelPriceHint")}</Text>
                  <Button compact icon={<Trash2 size={16} color={colors.danger} />} label={t("desktopSettings.removeModel")}
                    onPress={() => removeModel(model.key)} variant="danger" />
                </View>
                : null}
            </View>;
          })}
      </View>

      {editing
        ? <View style={[settingsStyles.card, styles.stack]}>
          {confirmDelete
            ? <>
              <Text style={settingsStyles.label}>{t("desktopSettings.deleteProviderConfirm", { name: provider.name || provider.id })}</Text>
              <View style={settingsStyles.actions}>
                <Button disabled={busy} label={t("desktopSettings.deleteProvider")} loading={busy}
                  onPress={() => void remove()} variant="danger" />
                <Button disabled={busy} label={t("chat.cancel")} onPress={() => setConfirmDelete(false)} variant="secondary" />
              </View>
            </>
            : <Button compact label={t("desktopSettings.deleteProvider")} onPress={() => setConfirmDelete(true)} variant="secondary" />}
        </View>
        : null}
    </ScrollView>
    <ActionMenu title={t("desktopSettings.apiType")} visible={pickingApi} onClose={() => setPickingApi(false)}
      actions={CUSTOM_PROVIDER_APIS.map(option => ({
        label: API_LABELS[option] ?? option,
        onPress: () => setApi(option),
      }))} />
  </>;
}

const useStyles = createThemedStyles(theme => StyleSheet.create({
  stack: { gap: spacing.md },
  modelsHeader: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  addModel: { minHeight: 32, justifyContent: "center", paddingHorizontal: spacing.sm, borderRadius: radius.sm },
  addModelLabel: { color: theme.colors.accent, fontSize: theme.font(13), fontWeight: "600" },
  model: { borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.md, overflow: "hidden" },
  modelHeader: { minHeight: 52, flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: spacing.md, padding: spacing.md },
  modelBody: { padding: spacing.md, borderTopWidth: 1, borderTopColor: theme.colors.line, backgroundColor: theme.colors.surfaceSubtle },
  priceGrid: { flexDirection: "row", flexWrap: "wrap", gap: spacing.md },
  priceCell: { flexGrow: 1, flexBasis: "45%" },
  levelChip: { minHeight: 36, justifyContent: "center", paddingHorizontal: spacing.md, borderWidth: 1, borderColor: theme.colors.line, borderRadius: radius.pill, backgroundColor: theme.colors.surface },
  levelChipSelected: { borderColor: theme.colors.accent, backgroundColor: theme.colors.accentSoft },
  levelChipLabel: { color: theme.colors.ink, fontSize: theme.font(13), fontWeight: "600" },
  levelChipLabelSelected: { color: theme.colors.accent },
}));
