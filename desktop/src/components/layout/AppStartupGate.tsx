import type { AppStartupSnapshot } from "./hooks/useAppStartup";
import { Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";

/** The Agent is ready here. Report account/configuration loading separately. */
export function AppStartupGate({ startup, onRetry }: { startup: Exclude<AppStartupSnapshot, { phase: "ready" }>; onRetry: () => void }) {
  const { t } = useTranslation("layout");
  return (
    <div className="flex h-full items-center justify-center bg-canvas px-6 text-center text-ink">
      <div aria-live="polite" className="flex max-w-lg flex-col items-center gap-3" role="status">
        {startup.phase === "pending" ? <Loader2 aria-hidden="true" className="size-6 animate-spin text-accent" /> : null}
        <p className="text-lg font-medium">{t(startup.phase === "pending" ? "appStartup.loading" : "appStartup.failed")}</p>
        {startup.phase === "failed" ? <p role="alert" className="text-sm text-ink-muted">{t(`appStartup.${startup.reason}`)}</p> : null}
        {startup.phase === "failed" ? <Button onClick={onRetry} variant="primary">{t("appStartup.retry")}</Button> : null}
      </div>
    </div>
  );
}
