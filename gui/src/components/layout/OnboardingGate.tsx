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
 * Onboarding gate shown on first launch when no provider is configured yet. It
 * guides the user toward two paths:
 *
 * 1. Sign in / sign up to FutureOS (primary, prominent button).
 * 2. Bring their own API key (secondary, subdued button) — opens Settings →
 *    Providers to add a custom provider.
 *
 * The language switcher lives in the top-right corner and is always visible.
 * Dev/test builds additionally show an environment switcher next to it.
 *
 * The gate clears automatically when any provider gains a key (via the
 * `future-auth-changed` event). The BYOK path clears it immediately.
 */
export interface OnboardingGateProps {
  onEnableBYOK: () => void;
}

export function OnboardingGate({ onEnableBYOK }: OnboardingGateProps) {
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
      await invokeCommand("set_future_environment", { environment: value });
    }
    catch {
      setSwitching(false);
    }
  }

  return (
    <div className="fixed inset-0 z-[60] flex flex-col items-center justify-center gap-6 bg-canvas px-6 text-center">
      {/* Top-right corner: language (always) + environment (dev only) */}
      <div className="absolute right-4 top-4 flex items-center gap-2">
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
        {isDev
          ? (
              <>
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
              </>
            )
          : null}
      </div>

      <div className="space-y-3">
        <h1 className="text-3xl font-semibold tracking-normal text-ink">{t("gate.title")}</h1>
        <p className="mx-auto max-w-md text-sm text-ink-muted">{t("gate.subtitle")}</p>
      </div>

      {failed
        ? <p className="max-w-md text-sm text-danger">{message ?? t("settings:futureLogin.failed")}</p>
        : null}

      <div className="flex flex-col items-center gap-3">
        <Button
          className="min-w-40"
          disabled={busy}
          leftIcon={busy ? <Loader2 className="size-4 animate-spin" /> : undefined}
          onClick={() => void begin()}
          variant="primary"
        >
          {busy ? t("gate.loggingIn") : failed ? t("gate.retry") : t("gate.login")}
        </Button>
        <p className="text-xs text-ink-muted">{t("gate.freeTrialHint")}</p>
        <div className="my-3 h-px w-48 bg-line" />
        <Button
          onClick={onEnableBYOK}
          size="sm"
          variant="secondary"
        >
          {t("gate.byok")}
        </Button>
      </div>
    </div>
  );
}
