import { useTranslation } from "react-i18next";
import { ScrollView, Text, View } from "react-native";
import { Button } from "../../components/Button";
import { useRemoteControls } from "../../remote/RemoteContext";
import type { RemoteBuiltinProvider, RemoteCustomProvider } from "../../remote/types";
import { ResourceStatus, SettingsLink, SettingsSection, settingsStyles } from "./SettingsPrimitives";
import { useDesktopResource } from "./useDesktopResource";

/** The account provider is signed in from the desktop; never editable here. */
const FUTURE_PROVIDER_ID = "future";

/**
 * Providers and their models, as configured on the paired desktop. Everything
 * shown (and written) is desktop state: the phone keeps no copy, and a key that
 * already exists is only ever reported as set/not set.
 */
export function ProvidersSettingsPage({ onOpenBuiltin, onOpenCustom }: {
  onOpenBuiltin(provider: RemoteBuiltinProvider): void;
  onOpenCustom(provider: RemoteCustomProvider | null): void;
}) {
  const { t } = useTranslation();
  const remote = useRemoteControls();
  const supported = remote.capabilities?.has("provider_management_v1") ?? false;
  const enabled = remote.desktopOnline && supported;
  const resource = useDesktopResource(remote.listProviders, remote.desktopSettingsRevision, enabled);
  const builtin = resource.data?.builtin ?? [];
  const custom = resource.data?.custom ?? [];
  const modelCount = (count: number) => t("desktopSettings.providerModelCount", { count });
  const keyState = (hasApiKey: boolean) => hasApiKey ? t("desktopSettings.providerKeySet") : t("desktopSettings.providerKeyMissing");

  return <ScrollView contentContainerStyle={settingsStyles.content} keyboardShouldPersistTaps="handled">
    <Text style={settingsStyles.description}>{t("desktopSettings.providersHint")}</Text>
    {!remote.desktopOnline || !supported
      ? <Text style={settingsStyles.description}>{t("desktopSettings.updateDesktop")}</Text>
      : null}
    <SettingsSection title={t("desktopSettings.builtinProviders")}>
      {builtin.map(provider => provider.id === FUTURE_PROVIDER_ID
        ? <View key={provider.id} style={settingsStyles.row}>
          <View style={settingsStyles.labelContainer}>
            <Text style={settingsStyles.label}>{provider.name}</Text>
            <Text style={settingsStyles.description}>{t("desktopSettings.providerManaged")}</Text>
          </View>
        </View>
        : <SettingsLink
          key={provider.id}
          label={provider.name}
          description={provider.requiresBaseUrl
            ? `${modelCount(provider.modelCount)} · ${t("desktopSettings.providerBaseUrlNeeded")}`
            : `${modelCount(provider.modelCount)} · ${keyState(provider.hasApiKey)}`}
          disabled={!enabled}
          onPress={() => onOpenBuiltin(provider)}
        />)}
      <ResourceStatus loading={resource.loading} failed={resource.failed} onReload={() => void resource.reload()} />
    </SettingsSection>
    <SettingsSection title={t("desktopSettings.customProviders")}>
      {custom.length === 0
        ? <Text style={settingsStyles.description}>{t("desktopSettings.noCustomProviders")}</Text>
        : custom.map(provider => <SettingsLink
          key={provider.id}
          label={provider.name}
          description={`${modelCount(provider.models.length)} · ${keyState(provider.hasApiKey)}`}
          disabled={!enabled}
          onPress={() => onOpenCustom(provider)}
        />)}
      <Button compact disabled={!enabled} label={t("desktopSettings.addProvider")} onPress={() => onOpenCustom(null)} variant="secondary" />
    </SettingsSection>
  </ScrollView>;
}
