import type { Language } from "../../i18n";
import { Loader2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useFutureLoginFlow } from "../../features/settings/useFutureLoginFlow";
import { getLanguage, LANGUAGE_LABELS, setLanguage, SUPPORTED_LANGUAGES } from "../../i18n";
import { getFutureEnvironment } from "../../integrations/agent/providers";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { Button } from "../ui/Button";
import { Select } from "../ui/Select";

type EnvironmentId = "production" | "test";

const ENVIRONMENTS: { id: EnvironmentId; labelKey: string }[] = [
  { id: "production", labelKey: "gate.envProduction" },
  { id: "test", labelKey: "gate.envTest" },
];

/**
 * Full-screen gate shown while the user is not signed in to a FutureOS account
 * (on launch, or after signing out / dropping the Future provider). It blocks the
 * whole app and offers a single path forward: start the device-code login, which
 * opens the browser directly. The language can be switched here because the rest
 * of the app (and its settings) is unreachable until signed in.
 *
 * No close affordance — signing in is mandatory. Success clears the gate via the
 * `future-auth-changed` event (see `useFutureSignedIn`), not via a callback.
 *
 * Dev/test builds additionally show an environment switcher (production / test)
 * in the top-right corner, since the settings page is unreachable behind the gate.
 */
export function ForceLoginGate() {
  const { t } = useTranslation("layout");
  const { phase, message, begin } = useFutureLoginFlow(() => {});
  const busy = phase === "starting" || phase === "waiting";
  const failed = phase === "denied" || phase === "expired" || phase === "error";
  const build = useBuildInfo();
  const isDev = build.data != null && !build.data.isRelease;

  const env = useAsyncResource(getFutureEnvironment, [], null);
  const [switching, setSwitching] = useState(false);

  const activeId = env.data?.environment;
  const envValue: EnvironmentId | "" = activeId === "test" || activeId === "production" ? activeId : "";

  async function handleEnvChange(value: EnvironmentId) {
    if (value === envValue || switching)
      return;
    setSwitching(true);
    try {
      // Backend pins the new base_url and restarts the app, so this normally
      // never resolves.
      await invokeCommand("set_future_environment", { environment: value });
    }
    catch {
      setSwitching(false);
    }
  }

  return (
    <div className="fixed inset-0 z-[60] flex flex-col items-center justify-center gap-6 bg-canvas px-6 text-center">
      {isDev
        ? (
            <div className="absolute right-4 top-4 flex items-center gap-2">
              <span className="text-xs text-ink-muted">{t("gate.envLabel")}</span>
              <Select
                disabled={switching || env.loading}
                onChange={e => void handleEnvChange(e.target.value as EnvironmentId)}
                size="sm"
                value={envValue}
                wrapperClassName="w-32"
              >
                {envValue === "" ? <option value="">{env.loading ? "..." : "custom"}</option> : null}
                {ENVIRONMENTS.map(item => (
                  <option key={item.id} value={item.id}>{t(item.labelKey)}</option>
                ))}
              </Select>
              {switching ? <Loader2 className="size-3.5 animate-spin text-ink-muted" /> : null}
            </div>
          )
        : null}

      <div className="space-y-3">
        <h1 className="text-3xl font-semibold tracking-normal text-ink">{t("gate.title")}</h1>
        <p className="mx-auto max-w-md text-sm text-ink-muted">{t("gate.subtitle")}</p>
      </div>

      {failed
        ? <p className="max-w-md text-sm text-danger">{message ?? t("settings:futureLogin.failed")}</p>
        : null}

      <div className="flex items-center gap-3">
        <Button
          className="min-w-32"
          disabled={busy}
          leftIcon={busy ? <Loader2 className="size-4 animate-spin" /> : undefined}
          onClick={() => void begin()}
          variant="primary"
        >
          {busy ? t("gate.loggingIn") : failed ? t("gate.retry") : t("gate.login")}
        </Button>
        <Select
          onChange={e => setLanguage(e.target.value as Language)}
          size="sm"
          value={getLanguage()}
          wrapperClassName="w-28"
        >
          {SUPPORTED_LANGUAGES.map(lang => (
            <option key={lang} value={lang}>{LANGUAGE_LABELS[lang]}</option>
          ))}
        </Select>
      </div>
    </div>
  );
}
