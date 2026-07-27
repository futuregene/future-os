import type { Language } from "../../i18n";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useFutureLoginFlow } from "../../features/settings/useFutureLoginFlow";
import { getLanguage, LANGUAGE_LABELS, setLanguage, SUPPORTED_LANGUAGES } from "../../i18n";
import { Button } from "../ui/Button";
import { Select } from "../ui/Select";

/**
 * Full-screen gate shown while the user is not signed in to a FutureOS account
 * (on launch, or after signing out / dropping the Future provider). It blocks the
 * whole app and offers a single path forward: start the device-code login, which
 * opens the browser directly. The language can be switched here because the rest
 * of the app (and its settings) is unreachable until signed in.
 *
 * No close affordance — signing in is mandatory. Success clears the gate via the
 * `future-auth-changed` event (see `useFutureSignedIn`), not via a callback.
 */
export function ForceLoginGate() {
  const { t } = useTranslation("layout");
  const { phase, message, begin } = useFutureLoginFlow(() => {});
  const busy = phase === "starting" || phase === "waiting";
  const failed = phase === "denied" || phase === "expired" || phase === "error";

  return (
    <div className="fixed inset-0 z-[60] flex flex-col items-center justify-center gap-6 bg-canvas px-6 text-center">
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
