import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Select } from "../../components/ui/Select";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { useBuildInfo } from "../../lib/useBuildInfo";
import { SettingsSection } from "./SettingsPrimitives";

type EnvironmentId = "production" | "test";

interface FutureEnvironment {
  /** `production` | `test` | `custom`. */
  environment: string;
  /** Resolved platform root currently in effect (no `/api`). */
  platformUrl: string;
}

// The platform URL for each environment is owned by the Tauri backend
// (`set_future_environment`); the frontend only sends the id.
const ENVIRONMENTS: { id: EnvironmentId; labelKey: string }[] = [
  { id: "production", labelKey: "reset.environmentProduction" },
  { id: "test", labelKey: "reset.environmentTest" },
];

function EnvironmentSection() {
  const { t } = useTranslation("settings");
  const current = useAsyncResource<FutureEnvironment | null>(
    () => invokeCommand<FutureEnvironment>("get_future_environment"),
    [],
    null,
  );
  const [selected, setSelected] = useState<EnvironmentId | "">("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const activeId = current.data?.environment;
  // Default the picker to the active environment once it loads.
  const fallback: EnvironmentId | "" = activeId === "test" || activeId === "production" ? activeId : "";
  const value: EnvironmentId | "" = selected || fallback;
  const changed = value !== "" && value !== activeId;

  async function handleSwitch() {
    // `changed` already guarantees a concrete environment id (not "").
    if (!changed) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // Backend pins the new base_url and restarts the app, so this normally
      // never resolves; an error here means the switch failed before restart.
      await invokeCommand("set_future_environment", { environment: value });
    }
    catch (switchError) {
      setError(switchError instanceof Error ? switchError.message : String(switchError));
      setBusy(false);
    }
  }

  return (
    <SettingsSection title={t("reset.environmentTitle")}>
      <div className="space-y-3 rounded-lg border border-line-soft p-4">
        <div>
          <div className="text-sm font-medium text-ink">{t("reset.switchEnvironment")}</div>
          <p className="mt-1 text-xs leading-5 text-ink-muted">
            {t("reset.switchEnvironmentDescription")}
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Select
            disabled={busy || current.loading}
            onChange={event => setSelected(event.target.value as EnvironmentId)}
            size="sm"
            value={value}
            wrapperClassName="w-44"
          >
            {value === "" ? <option value="">{current.loading ? t("reset.loading") : t("reset.customEnvironment")}</option> : null}
            {ENVIRONMENTS.map(env => (
              <option key={env.id} value={env.id}>{t(env.labelKey)}</option>
            ))}
          </Select>
          <Button disabled={!changed || busy} onClick={() => void handleSwitch()} size="sm" variant="primary">
            {busy ? t("reset.switching") : t("reset.switch")}
          </Button>
        </div>

        <p className="text-xs text-ink-muted">
          {t("reset.current")}
          {current.loading ? t("reset.loading") : (current.data?.platformUrl ?? t("reset.unknown"))}
        </p>

        {current.error ? <p className="text-xs text-danger">{current.error}</p> : null}
        {error ? <p className="text-xs text-danger">{error}</p> : null}
      </div>
    </SettingsSection>
  );
}

export function ResetPage() {
  const { t } = useTranslation("settings");
  // The environment switcher is a dev-build affordance; release builds are
  // production-locked (the backend also refuses non-production switches).
  const build = useBuildInfo();
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
      {build.data && !build.data.isRelease ? <EnvironmentSection /> : null}

      <SettingsSection title={t("reset.resetTitle")}>
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
