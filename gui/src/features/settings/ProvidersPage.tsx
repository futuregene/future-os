import type { BuiltinProvider, CustomProvider, ProvidersView } from "../../integrations/agent/providers";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge } from "../../components/ui/Badge";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { Field } from "../../components/ui/Field";
import { TextInput } from "../../components/ui/TextInput";
import {
  deleteCustomProvider,
  listAgentProviders,
  logoutFutureProvider,
  updateBuiltinProviderKey,
  upsertCustomProvider,
} from "../../integrations/agent/providers";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { CustomProviderDialog } from "./CustomProviderDialog";
import { FutureLoginDialog } from "./FutureLoginDialog";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

const DEFAULT_BUILTIN_PROVIDER_IDS = [
  "future",
  "deepseek",
  "kimi-coding",
  "minimax-cn",
  "moonshotai-cn",
  "zhipuai",
  "anthropic",
  "openai",
  "google",
];

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
  const [editingBuiltinKey, setEditingBuiltinKey] = useState<BuiltinProvider | null>(null);
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
  const [loginOpen, setLoginOpen] = useState(false);
  const [confirmingLogout, setConfirmingLogout] = useState(false);
  const [showMoreBuiltin, setShowMoreBuiltin] = useState(false);
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

  async function handleBuiltinKey(provider: BuiltinProvider, apiKey: string | null) {
    const view = await updateBuiltinProviderKey({ apiKey, id: provider.id });
    setProviders(view);
    setEditingBuiltinKey(null);
    setHint(apiKey ? t("providers.keySaved", { provider: provider.name }) : t("providers.keyCleared", { provider: provider.name }));
  }

  function handleAuthorized() {
    setLoginOpen(false);
    reload();
    setHint(t("providers.connected"));
  }

  if (loading) {
    return <p className="text-sm text-ink-muted">{t("providers.loading")}</p>;
  }

  const builtinProviders = providers?.builtin ?? [];
  const defaultBuiltinProviders = DEFAULT_BUILTIN_PROVIDER_IDS
    .map(id => builtinProviders.find(provider => provider.id === id))
    .filter((provider): provider is BuiltinProvider => Boolean(provider));
  const defaultBuiltinIds = new Set(defaultBuiltinProviders.map(provider => provider.id));
  const hiddenBuiltinProviders = builtinProviders.filter(provider => !defaultBuiltinIds.has(provider.id));
  const visibleBuiltinProviders = showMoreBuiltin
    ? [...defaultBuiltinProviders, ...hiddenBuiltinProviders]
    : defaultBuiltinProviders;

  return (
    <div className="space-y-6">
      {error ? <p className="text-sm text-danger">{error}</p> : null}
      {hint ? <p className="text-sm text-ink-soft">{hint}</p> : null}

      <SettingsSection title={t("providers.builtinTitle")}>
        <SettingsList>
          {visibleBuiltinProviders.map(provider => (
            <SettingsRow
              key={provider.id}
              title={provider.name}
              description={t("providers.builtinModelsCount", { count: provider.modelCount })}
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
                    <div className="flex items-center gap-2">
                      <Badge tone={provider.hasApiKey ? "success" : "neutral"}>
                        {provider.hasApiKey ? t("providers.hasApiKey") : t("providers.noApiKey")}
                      </Badge>
                      <Button
                        onClick={() => {
                          setHint(null);
                          setEditingBuiltinKey(provider);
                        }}
                        size="sm"
                        variant="secondary"
                      >
                        {provider.hasApiKey ? t("providers.updateKey") : t("providers.setKey")}
                      </Button>
                    </div>
                  )}
            </SettingsRow>
          ))}
        </SettingsList>
        {hiddenBuiltinProviders.length > 0
          ? (
              <Button
                onClick={() => setShowMoreBuiltin(value => !value)}
                size="sm"
                variant="secondary"
              >
                {showMoreBuiltin
                  ? t("providers.hideMoreBuiltin")
                  : t("providers.showMoreBuiltin", { count: hiddenBuiltinProviders.length })}
              </Button>
            )
          : null}
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

      <BuiltinProviderKeyDialog
        onClose={() => setEditingBuiltinKey(null)}
        onSubmit={apiKey => editingBuiltinKey ? handleBuiltinKey(editingBuiltinKey, apiKey) : Promise.resolve()}
        open={Boolean(editingBuiltinKey)}
        provider={editingBuiltinKey}
      />
    </div>
  );
}

function BuiltinProviderKeyDialog({
  onClose,
  onSubmit,
  open,
  provider,
}: {
  onClose: () => void;
  onSubmit: (apiKey: string | null) => Promise<void>;
  open: boolean;
  provider: BuiltinProvider | null;
}) {
  const { t } = useTranslation("settings");
  const [apiKey, setApiKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) {
      return;
    }
    setApiKey("");
    setError(null);
    setSaving(false);
  }, [open, provider?.id]);

  async function submit(nextKey: string | null) {
    const trimmed = nextKey?.trim() ?? null;
    if (nextKey !== null && !trimmed) {
      setError(t("providers.keyRequired"));
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await onSubmit(trimmed);
    }
    catch (submitError) {
      setError(submitError instanceof Error ? submitError.message : String(submitError));
      setSaving(false);
    }
  }

  return (
    <Dialog
      className="max-w-md"
      onClose={onClose}
      open={open}
      title={t("providers.keyDialogTitle", { provider: provider?.name ?? "" })}
      description={t("providers.keyDialogDescription")}
      footer={(
        <>
          {provider?.hasApiKey
            ? (
                <Button disabled={saving} onClick={() => void submit(null)} variant="secondary">
                  {t("providers.clearKey")}
                </Button>
              )
            : null}
          <Button onClick={onClose} variant="secondary">{t("providers.cancel")}</Button>
          <Button disabled={saving} onClick={() => void submit(apiKey)} variant="primary">
            {saving ? t("providers.savingKey") : t("providers.saveKey")}
          </Button>
        </>
      )}
    >
      <div className="space-y-3">
        <Field label={t("customProvider.apiKeyLabel")}>
          <TextInput
            autoFocus
            onChange={event => setApiKey(event.target.value)}
            placeholder={t("customProvider.apiKeyPlaceholder")}
            type="password"
            value={apiKey}
          />
        </Field>
        {error ? <p className="text-sm text-danger">{error}</p> : null}
      </div>
    </Dialog>
  );
}
