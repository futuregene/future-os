import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { Button } from "../../components/Button";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { RemoteBuiltinProvider } from "../../remote/types";
import { colors, spacing } from "../../theme/tokens";
import { SettingsField, settingsStyles } from "./SettingsPrimitives";

/** Marker left in a catalog base URL that the user must replace (Azure et al.). */
const BASE_URL_PLACEHOLDER = "YOUR_RESOURCE";

/** Desktop-authored validation text is shown as-is, like in the desktop form. */
function detail(failure: unknown): string {
  return failure instanceof Error ? failure.message : String(failure);
}

/**
 * One built-in provider's API key and (when the catalog needs it) Base URL.
 * Mirrors the desktop dialog: a provider whose catalog address is a placeholder
 * requires the Base URL and treats the key as optional, while every other
 * provider requires the key. Saving is one atomic desktop write.
 */
export function ProviderKeyPage({ provider, onSaved }: {
  provider: RemoteBuiltinProvider;
  /** Leave the page once the desktop confirmed the write. */
  onSaved(): void;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  // A key is never sent back to a phone, so the field always starts empty.
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState(
    provider.baseUrl.includes(BASE_URL_PLACEHOLDER) ? "" : provider.baseUrl,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async (payload: { apiKey?: string | null; updateApiKey: boolean; baseUrl?: string }) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await remote.updateBuiltinProvider({ id: provider.id, ...payload });
      onSaved();
    }
    catch (failure) {
      setError(detail(failure));
    }
    finally {
      setBusy(false);
    }
  };

  const save = () => {
    const key = apiKey.trim();
    const url = baseUrl.trim();
    if (provider.requiresBaseUrl) {
      if (!url) {
        setError(t("desktopSettings.baseUrlRequired"));
        return;
      }
      // The key is optional here: an empty field leaves the stored key alone.
      void run(key ? { apiKey: key, baseUrl: url, updateApiKey: true } : { baseUrl: url, updateApiKey: false });
      return;
    }
    if (!key) {
      setError(t("desktopSettings.apiKeyRequired"));
      return;
    }
    void run({ apiKey: key, updateApiKey: true });
  };

  return <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
    <View style={[settingsStyles.card, styles.stack]}>
      {provider.requiresBaseUrl
        ? <SettingsField label={t("desktopSettings.baseUrl")} hint={t("desktopSettings.baseUrlHint")}>
          <TextInput accessibilityLabel={t("desktopSettings.baseUrl")} autoCapitalize="none" autoCorrect={false}
            keyboardType="url" onChangeText={setBaseUrl} placeholder="https://…"
            placeholderTextColor={colors.inkMuted} style={settingsStyles.input} value={baseUrl} />
        </SettingsField>
        : null}
      <SettingsField
        error={error ?? undefined}
        hint={provider.requiresBaseUrl
          ? t("desktopSettings.apiKeyOptionalHint")
          : t("desktopSettings.apiKeyHint")}
        label={t("desktopSettings.apiKey")}
      >
        <TextInput accessibilityLabel={t("desktopSettings.apiKey")} autoCapitalize="none" autoCorrect={false}
          onChangeText={setApiKey} placeholder={provider.hasApiKey ? t("desktopSettings.apiKeyKeep") : t("desktopSettings.apiKeyPlaceholder")}
          placeholderTextColor={colors.inkMuted} secureTextEntry style={settingsStyles.input} value={apiKey} />
      </SettingsField>
      <Text style={settingsStyles.description}>
        {provider.hasApiKey ? t("desktopSettings.apiKeyStored") : t("desktopSettings.apiKeyMissing")}
      </Text>
      <View style={settingsStyles.actions}>
        <Button disabled={busy} label={t("desktopSettings.save")} loading={busy} onPress={save} variant="primary" />
        {provider.hasApiKey
          ? <Button disabled={busy} label={t("desktopSettings.clearApiKey")}
            onPress={() => void run({ apiKey: null, updateApiKey: true })} variant="danger" />
          : null}
      </View>
    </View>
  </ScrollView>;
}

const styles = StyleSheet.create({
  stack: { gap: spacing.md },
});
