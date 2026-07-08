import type { BuiltinProvider } from "../../integrations/agent/providers";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { Field } from "../../components/ui/Field";
import { TextInput } from "../../components/ui/TextInput";
import { errorMessage } from "../../lib/errors";

/** Marker left in a catalog base URL that the user must replace (Azure et al.). */
const BASE_URL_PLACEHOLDER = "YOUR_RESOURCE";

export function BuiltinProviderKeyDialog({
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
      setError(errorMessage(submitError));
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
