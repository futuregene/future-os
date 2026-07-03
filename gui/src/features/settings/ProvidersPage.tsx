import type { CustomProvider, ProvidersView } from "../../integrations/agent/providers";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import {
  deleteCustomProvider,
  listAgentProviders,
  logoutFutureProvider,
  upsertCustomProvider,
} from "../../integrations/agent/providers";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { CustomProviderDialog } from "./CustomProviderDialog";
import { FutureLoginDialog } from "./FutureLoginDialog";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

export function ProvidersPage() {
  const { t } = useTranslation("settings");
  const { data: loadedProviders, loading, error, reload } = useAsyncResource<ProvidersView | null>(
    listAgentProviders,
    [],
    null,
  );
  // Mirror the loaded view locally so mutations (delete/logout/upsert) can apply
  // their returned view optimistically without waiting for a refetch.
  const [providers, setProviders] = useState<ProvidersView | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<CustomProvider | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
  const [loginOpen, setLoginOpen] = useState(false);
  const [confirmingLogout, setConfirmingLogout] = useState(false);
  const [hint, setHint] = useState<string | null>(null);

  useEffect(() => {
    if (loadedProviders)
      setProviders(loadedProviders);
  }, [loadedProviders]);

  async function handleDelete(id: string) {
    const view = await deleteCustomProvider(id);
    setProviders(view);
    setConfirmingDelete(null);
  }

  async function handleLogout() {
    const view = await logoutFutureProvider();
    setProviders(view);
    setConfirmingLogout(false);
    setHint(null);
  }

  function handleAuthorized() {
    setLoginOpen(false);
    reload();
    setHint(t("providers.connected"));
  }

  if (loading) {
    return <p className="text-sm text-ink-muted">{t("providers.loading")}</p>;
  }

  return (
    <div className="space-y-6">
      {error ? <p className="text-sm text-danger">{error}</p> : null}
      {hint ? <p className="text-sm text-ink-soft">{hint}</p> : null}

      <SettingsSection title={t("providers.builtinTitle")}>
        <SettingsList>
          {(providers?.builtin ?? []).map(provider => (
            <SettingsRow
              key={provider.id}
              title={provider.name}
              description={provider.baseUrl}
            >
              {provider.id === "future"
                ? (
                    <div className="flex items-center gap-2">
                      {confirmingLogout && provider.hasApiKey
                        ? (
                            <>
                              <span className="text-xs text-ink-muted">{t("providers.confirmLogout")}</span>
                              <Button onClick={() => void handleLogout()} size="sm" variant="danger">{t("providers.logoutConfirm")}</Button>
                              <Button onClick={() => setConfirmingLogout(false)} size="sm" variant="secondary">{t("providers.cancel")}</Button>
                            </>
                          )
                        : (
                            <>
                              <Badge tone={provider.hasApiKey ? "success" : "neutral"}>
                                {provider.hasApiKey ? t("providers.hasApiKey") : t("providers.noApiKey")}
                              </Badge>
                              <Button
                                onClick={() => {
                                  setHint(null);
                                  setLoginOpen(true);
                                }}
                                size="sm"
                                variant="secondary"
                              >
                                {provider.hasApiKey ? t("providers.reLogin") : t("providers.connect")}
                              </Button>
                              {provider.hasApiKey
                                ? (
                                    <Button
                                      className="text-ink-soft hover:text-danger"
                                      onClick={() => setConfirmingLogout(true)}
                                      size="sm"
                                      variant="secondary"
                                    >
                                      {t("providers.logout")}
                                    </Button>
                                  )
                                : null}
                            </>
                          )}
                    </div>
                  )
                : (
                    <Badge tone={provider.hasApiKey ? "success" : "neutral"}>
                      {provider.hasApiKey ? t("providers.hasApiKey") : t("providers.noApiKey")}
                    </Badge>
                  )}
            </SettingsRow>
          ))}
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        title={t("providers.customTitle")}
        action={(
          <Button
            onClick={() => {
              setEditing(null);
              setDialogOpen(true);
            }}
            size="sm"
            variant="secondary"
          >
            {t("providers.addCustom")}
          </Button>
        )}
      >
        {providers && providers.custom.length > 0
          ? (
              <SettingsList>
                {providers.custom.map(provider => (
                  <SettingsRow
                    key={provider.id}
                    title={provider.name}
                    description={t("providers.modelsCount", { baseUrl: provider.baseUrl, count: provider.models.length })}
                  >
                    {confirmingDelete === provider.id
                      ? (
                          <div className="flex items-center gap-2">
                            <span className="text-xs text-ink-muted">{t("providers.confirmRemove")}</span>
                            <Button onClick={() => void handleDelete(provider.id)} size="sm" variant="danger">
                              {t("providers.remove")}
                            </Button>
                            <Button onClick={() => setConfirmingDelete(null)} size="sm" variant="secondary">
                              {t("providers.cancel")}
                            </Button>
                          </div>
                        )
                      : (
                          <div className="flex items-center gap-2">
                            <Button
                              onClick={() => {
                                setEditing(provider);
                                setDialogOpen(true);
                              }}
                              size="sm"
                              variant="secondary"
                            >
                              {t("providers.edit")}
                            </Button>
                            <Button
                              className="text-ink-soft hover:text-danger"
                              onClick={() => setConfirmingDelete(provider.id)}
                              size="sm"
                              variant="secondary"
                            >
                              {t("providers.remove")}
                            </Button>
                          </div>
                        )}
                  </SettingsRow>
                ))}
              </SettingsList>
            )
          : <p className="text-sm text-ink-muted">{t("providers.noCustom")}</p>}
      </SettingsSection>

      <CustomProviderDialog
        existing={[
          ...(providers?.builtin ?? []).map(p => ({ id: p.id, name: p.name })),
          ...(providers?.custom ?? []).map(p => ({ id: p.id, name: p.name })),
        ]}
        initial={editing}
        onClose={() => setDialogOpen(false)}
        onSubmit={async (input) => {
          const view = await upsertCustomProvider(input);
          setProviders(view);
        }}
        open={dialogOpen}
      />

      <FutureLoginDialog
        onAuthorized={() => void handleAuthorized()}
        onClose={() => setLoginOpen(false)}
        open={loginOpen}
      />
    </div>
  );
}
