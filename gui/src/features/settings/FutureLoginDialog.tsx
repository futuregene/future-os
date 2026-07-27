import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";
import { copyText } from "../../lib/clipboard";
import { useFutureLoginFlow } from "./useFutureLoginFlow";

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
  const [copied, setCopied] = useState(false);
  // begin/cancel are stable (useCallback), so the open-lifecycle effect only
  // re-runs when `open` changes.
  const { phase, message, start, begin, cancel } = useFutureLoginFlow(onAuthorized);

  // Open → start a fresh attempt; close → cancel any in-flight poll so a late
  // "authorized" can't fire after the dialog is gone.
  useEffect(() => {
    if (open) {
      setCopied(false);
      void begin();
    }
    else {
      cancel();
    }
  }, [open, begin, cancel]);

  async function handleCopyLink() {
    if (!start)
      return;
    await copyText(start.verificationUriComplete);
    setCopied(true);
  }

  const failed = phase === "denied" || phase === "expired" || phase === "error";

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
          {failed
            ? <Button onClick={() => void begin()} variant="primary">{t("futureLogin.retry")}</Button>
            : null}
        </>
      )}
    >
      <div className="space-y-4">
        {phase === "starting" ? <p className="text-sm text-ink-muted">{t("futureLogin.gettingDeviceCode")}</p> : null}

        {phase === "waiting" && start
          ? (
              <div className="space-y-3">
                <Button onClick={() => void handleCopyLink()} size="sm" variant="secondary">
                  {copied ? t("futureLogin.copied") : t("futureLogin.copyLink")}
                </Button>
                <p className="text-sm text-ink-muted">{t("futureLogin.waiting")}</p>
              </div>
            )
          : null}

        {failed
          ? <p className="text-sm text-danger">{message ?? t("futureLogin.connectFailed")}</p>
          : null}
      </div>
    </Dialog>
  );
}
