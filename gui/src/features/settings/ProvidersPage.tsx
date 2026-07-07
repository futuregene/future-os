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
  setBuiltinProviderBaseUrl,
  updateBuiltinProviderKey,
  upsertCustomProvider,
} from "../../integrations/agent/providers";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { CustomProviderDialog } from "./CustomProviderDialog";
import { FutureLoginDialog } from "./FutureLoginDialog";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

/** Marker left in a catalog base URL that the user must replace (Azure et al.). */
const BASE_URL_PLACEHOLDER = "YOUR_RESOURCE";

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

export function ProvidersPage({
  onProvidersChanged,
}: {
  /**
   * Called after any mutation that changes the available model set, so the
   * Models tab (fed by the agent's `list_models`) refreshes immediately.
   */
  onProvidersChanged?: () => void;
} = {}) {
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
    onProvidersChanged?.();
  }

  async function handleLogout() {
    const view = await logoutFutureProvider();
    setProviders(view);
    setConfirmingLogout(false);
    setHint(null);
    onProvidersChanged?.();
  }

  async function handleBuiltinSubmit(
    provider: BuiltinProvider,
    payload: { apiKey?: string | null; baseUrl?: string },
  ) {
    // Base URL first, then key; each returns the fresh view, so keep the last.
    let view = null;
    if (payload.baseUrl !== undefined) {
      view = await setBuiltinProviderBaseUrl({ baseUrl: payload.baseUrl, id: provider.id });
    }
    if (payload.apiKey !== undefined) {
      view = await updateBuiltinProviderKey({ apiKey: payload.apiKey, id: provider.id });
    }
    if (view) {
      setProviders(view);
    }
    setEditingBuiltinKey(null);
    const cleared = payload.apiKey === null;
    setHint(
      cleared
        ? t("providers.keyCleared", { provider: provider.name })
        : t("providers.keySaved", { provider: provider.name }),
    );
    onProvidersChanged?.();
  }

  function handleAuthorized() {
    setLoginOpen(false);
    reload();
    setHint(t("providers.connected"));
    onProvidersChanged?.();
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
                                {provider.hasApiKey ? t("providers.loggedIn") : t("providers.loggedOut")}
                              </Badge>
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
                                : (
                                    <Button
                                      onClick={() => {
                                        setHint(null);
                                        setLoginOpen(true);
                                      }}
                                      size="sm"
                                      variant="secondary"
                                    >
                                      {t("providers.connect")}
                                    </Button>
                                  )}
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
                        {t("providers.set")}
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
          onProvidersChanged?.();
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
        onSubmit={payload => editingBuiltinKey ? handleBuiltinSubmit(editingBuiltinKey, payload) : Promise.resolve()}
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
  onSubmit: (payload: { apiKey?: string | null; baseUrl?: string }) => Promise<void>;
  open: boolean;
  provider: BuiltinProvider | null;
}) {
  const { t } = useTranslation("settings");
  const requiresBaseUrl = Boolean(provider?.requiresBaseUrl);
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!open) {
      return;
    }
    setApiKey("");
    // Prefill the current override, but not the unfilled placeholder.
    const current = provider?.baseUrl ?? "";
    setBaseUrl(current.includes(BASE_URL_PLACEHOLDER) ? "" : current);
    setError(null);
    setSaving(false);
  }, [open, provider?.id, provider?.baseUrl]);

  async function run(payload: { apiKey?: string | null; baseUrl?: string }) {
    setSaving(true);
    setError(null);
    try {
      await onSubmit(payload);
    }
    catch (submitError) {
      setError(submitError instanceof Error ? submitError.message : String(submitError));
      setSaving(false);
    }
  }

  function save() {
    const trimmedKey = apiKey.trim();
    const trimmedBaseUrl = baseUrl.trim();
    const payload: { apiKey?: string | null; baseUrl?: string } = {};

    if (requiresBaseUrl) {
      if (!trimmedBaseUrl) {
        setError(t("providers.baseUrlRequired"));
        return;
      }
      payload.baseUrl = trimmedBaseUrl;
      // Key is optional here — only touch it when the user typed one.
      if (trimmedKey) {
        payload.apiKey = trimmedKey;
      }
    }
    else {
      if (!trimmedKey) {
        setError(t("providers.keyRequired"));
        return;
      }
      payload.apiKey = trimmedKey;
    }
    void run(payload);
  }

  return (
    <Dialog
      className="max-w-md"
      onClose={onClose}
      open={open}
      title={t("providers.keyDialogTitle", { provider: provider?.name ?? "" })}
      description={requiresBaseUrl ? t("providers.baseUrlDialogDescription") : t("providers.keyDialogDescription")}
      footer={(
        <>
          {provider?.hasApiKey
            ? (
                <Button disabled={saving} onClick={() => void run({ apiKey: null })} variant="secondary">
                  {t("providers.clearKey")}
                </Button>
              )
            : null}
          <Button onClick={onClose} variant="secondary">{t("providers.cancel")}</Button>
          <Button disabled={saving} onClick={save} variant="primary">
            {saving ? t("providers.savingKey") : t("providers.saveKey")}
          </Button>
        </>
      )}
    >
      <div className="space-y-3">
        {requiresBaseUrl
          ? (
              <Field label={t("providers.baseUrlLabel")}>
                <TextInput
                  autoFocus
                  onChange={event => setBaseUrl(event.target.value)}
                  placeholder={provider?.baseUrl}
                  value={baseUrl}
                />
              </Field>
            )
          : null}
        <Field label={requiresBaseUrl ? t("providers.apiKeyOptionalLabel") : t("customProvider.apiKeyLabel")}>
          <TextInput
            autoFocus={!requiresBaseUrl}
            onChange={event => setApiKey(event.target.value)}
            placeholder={requiresBaseUrl && provider?.hasApiKey ? t("providers.apiKeyKeepPlaceholder") : t("customProvider.apiKeyPlaceholder")}
            type="password"
            value={apiKey}
          />
        </Field>
        {error ? <p className="text-sm text-danger">{error}</p> : null}
      </div>
    </Dialog>
  );
}
