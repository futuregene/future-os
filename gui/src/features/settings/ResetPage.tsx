import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { SettingsSection } from "./SettingsPrimitives";

export function ResetPage() {
  const { t } = useTranslation("settings");
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleClear() {
    setBusy(true);
    setError(null);
    try {
      // Backend clears local data and restarts the app, so this normally never
      // resolves; an error here means the wipe failed before the restart.
      await invokeCommand("clear_app_data");
    }
    catch (clearError) {
      setError(clearError instanceof Error ? clearError.message : String(clearError));
      setBusy(false);
      setConfirming(false);
    }
  }

  return (
    <div className="space-y-6">
      <SettingsSection>
        <div className="space-y-3 rounded-lg border border-line-soft p-4">
          <div>
            <div className="text-sm font-medium text-ink">{t("reset.clearData")}</div>
            <p className="mt-1 text-xs leading-5 text-ink-muted">
              {t("reset.clearDataDescription")}
            </p>
          </div>

          {error ? <p className="text-xs text-danger">{error}</p> : null}

          {confirming
            ? (
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-xs text-ink-muted">{t("reset.confirmClear")}</span>
                  <Button disabled={busy} onClick={() => void handleClear()} size="sm" variant="danger">
                    {busy ? t("reset.clearing") : t("reset.confirmClearButton")}
                  </Button>
                  <Button disabled={busy} onClick={() => setConfirming(false)} size="sm" variant="secondary">
                    {t("reset.cancel")}
                  </Button>
                </div>
              )
            : (
                <Button onClick={() => setConfirming(true)} size="sm" variant="danger">
                  {t("reset.clearData")}
                </Button>
              )}
        </div>
      </SettingsSection>
    </div>
  );
}
