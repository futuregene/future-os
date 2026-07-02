import type { FutureLoginStart } from "../../integrations/agent/providers";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { pollFutureLogin, startFutureLogin } from "../../integrations/agent/providers";
import { copyText } from "../../lib/clipboard";
import { usePolling } from "../../lib/usePolling";

type Phase = "starting" | "waiting" | "denied" | "expired" | "error";

const SLOW_DOWN_STEP_MS = 5000;
// Poll faster than the server's suggested interval for snappier "authorized"
// detection; if the server pushes back with `slow_down` we back off (+5s).
const FAST_POLL_MS = 2000;

export function FutureLoginDialog({
  open,
  onClose,
  onAuthorized,
}: {
  open: boolean;
  onClose: () => void;
  /** Called once login succeeds; parent refreshes providers and closes. */
  onAuthorized: () => void;
}) {
  const { t } = useTranslation("settings");
  const [phase, setPhase] = useState<Phase>("starting");
  const [start, setStart] = useState<FutureLoginStart | null>(null);
  const [pollIntervalMs, setPollIntervalMs] = useState(FAST_POLL_MS);
  const [message, setMessage] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  // Latest login attempt id: a poll response is discarded if a newer attempt
  // (retry) started while it was in flight (usePolling does not cancel in-flight
  // async). Also gates the per-attempt expiry deadline.
  const attemptRef = useRef(0);
  const deadlineRef = useRef(0);

  const begin = useCallback(async () => {
    const attempt = attemptRef.current + 1;
    attemptRef.current = attempt;
    setPhase("starting");
    setMessage(null);
    setStart(null);
    setCopied(false);
    try {
      const next = await startFutureLogin();
      if (attempt !== attemptRef.current)
        return;
      setStart(next);
      // Start snappy; respect the server interval only if it asks for slower.
      setPollIntervalMs(Math.min(Math.max(1, next.interval) * 1000, FAST_POLL_MS));
      deadlineRef.current = Date.now() + next.expiresIn * 1000;
      setPhase("waiting");
    }
    catch (error) {
      if (attempt !== attemptRef.current)
        return;
      setMessage(errorText(error));
      setPhase("error");
    }
  }, []);

  useEffect(() => {
    if (open)
      void begin();
    // Bump the attempt id on close so any in-flight poll is ignored.
    else
      attemptRef.current += 1;
  }, [open, begin]);

  usePolling(
    async () => {
      const current = start;
      if (!current)
        return;
      const attempt = attemptRef.current;
      if (Date.now() > deadlineRef.current) {
        // Invalidate any in-flight poll so a late "authorized" can't slip past
        // expiry and fire onAuthorized.
        attemptRef.current += 1;
        setPhase("expired");
        setMessage(t("futureLogin.expired"));
        return;
      }

      let result;
      try {
        result = await pollFutureLogin(current.deviceCode);
      }
      catch (error) {
        if (attempt !== attemptRef.current)
          return;
        setMessage(errorText(error));
        setPhase("error");
        return;
      }
      if (attempt !== attemptRef.current)
        return;

      switch (result.status) {
        case "authorized":
          // Invalidate further polls before handing off to the parent.
          attemptRef.current += 1;
          onAuthorized();
          break;
        case "pending":
          break;
        case "slow_down":
          setPollIntervalMs(ms => ms + SLOW_DOWN_STEP_MS);
          break;
        case "denied":
          setMessage(result.message ?? t("futureLogin.denied"));
          setPhase("denied");
          break;
        case "expired":
          setMessage(result.message ?? t("futureLogin.expired"));
          setPhase("expired");
          break;
        default:
          setMessage(result.message ?? t("futureLogin.failed"));
          setPhase("error");
          break;
      }
    },
    pollIntervalMs,
    { enabled: open && phase === "waiting" && start !== null, deps: [phase, start, pollIntervalMs] },
  );

  async function handleCopyLink() {
    if (!start)
      return;
    await copyText(start.verificationUriComplete);
    setCopied(true);
  }

  return (
    <Dialog
      className="max-w-md"
      onClose={onClose}
      open={open}
      title={t("futureLogin.title")}
      description={t("futureLogin.description")}
      footer={(
        <>
          <Button onClick={onClose} variant="secondary">{t("futureLogin.cancel")}</Button>
          {phase === "denied" || phase === "expired" || phase === "error"
            ? <Button onClick={() => void begin()} variant="primary">{t("futureLogin.retry")}</Button>
            : null}
        </>
      )}
    >
      <div className="space-y-4">
        {phase === "starting" ? <p className="text-sm text-ink-muted">{t("futureLogin.gettingDeviceCode")}</p> : null}

        {phase === "waiting" && start
          ? (
              <>
                <div className="space-y-1">
                  <span className="text-xs font-medium text-ink-muted">{t("futureLogin.verifyCode")}</span>
                  <div className="flex items-center gap-3">
                    <code className="select-all rounded-md bg-surface-subtle px-3 py-2 font-mono text-2xl font-semibold tracking-[0.2em] text-ink">
                      {start.userCode}
                    </code>
                    <Button onClick={() => void copyText(start.userCode)} size="sm" variant="secondary">
                      {t("futureLogin.copyCode")}
                    </Button>
                  </div>
                </div>
                <div className="space-y-1">
                  <span className="text-xs font-medium text-ink-muted">{t("futureLogin.authLink")}</span>
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate rounded-md bg-surface-subtle px-2 py-1.5 text-sm text-ink-soft" title={start.verificationUriComplete}>
                      {start.verificationUriComplete}
                    </span>
                    <Button onClick={() => void handleCopyLink()} size="sm" variant="secondary">
                      {copied ? t("futureLogin.copied") : t("futureLogin.copyLink")}
                    </Button>
                  </div>
                </div>
                <p className="text-sm text-ink-muted">{t("futureLogin.waiting")}</p>
              </>
            )
          : null}

        {phase === "denied" || phase === "expired" || phase === "error"
          ? <p className="text-sm text-danger">{message ?? t("futureLogin.connectFailed")}</p>
          : null}
      </div>
    </Dialog>
  );
}

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
