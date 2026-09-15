import type { FutureBalanceStatus, FutureSessionStatus } from "../../components/layout/hooks/useFutureAccount";
import type { FutureEnvironment } from "../../integrations/agent/providers";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { getFutureEnvironment, logoutFutureProvider } from "../../integrations/agent/providers";
import { openExternalUrl } from "../../integrations/storage/files";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

/**
 * Account page driven by the app-wide, platform-verified session state.
 */
export function AccountPage({
  balance,
  balanceStatus,
  communityEdition,
  email: accountEmail,
  sessionStatus,
  onRefreshAuth,
  onRefreshBalance,
}: {
  balance: number | null;
  balanceStatus: FutureBalanceStatus;
  communityEdition: boolean;
  email: string | null;
  sessionStatus: FutureSessionStatus;
  onRefreshAuth: () => void;
  onRefreshBalance: () => void;
}) {
  const { t } = useTranslation("settings");
  // The platform host follows the active environment (test vs production).
  const environment = useAsyncResource<FutureEnvironment | null>(
    getFutureEnvironment,
    [],
    null,
  );
  const [confirmingLogout, setConfirmingLogout] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const loggedIn = sessionStatus === "authenticated" || sessionStatus === "unavailable";

  // Credits can change between settings visits; refresh them once when this
  // page opens without touching the cached account profile.
  useEffect(() => {
    if (loggedIn && !communityEdition)
      onRefreshBalance();
  }, [communityEdition, loggedIn, onRefreshBalance]);

  // AppShell owns the single account query and passes its cached values down.
  // While an authenticated profile is still loading, keep the description
  // empty instead of flashing a generic "Signed in" label first.
  const signedInLabel = accountEmail ?? "";
  const platformUrl = environment.data?.platformUrl;

  async function handleRecharge() {
    if (!platformUrl)
      return;
    await openExternalUrl(`${platformUrl}/platform/#recharge`);
  }

  async function handleLogout() {
    setActionError(null);
    try {
      // logoutFutureProvider clears the profile cache internally.
      await logoutFutureProvider();
      setConfirmingLogout(false);
    }
    catch (error) {
      setActionError(errorMessage(error));
    }
  }

  async function handleOpenAccount() {
    const platformUrl = environment.data?.platformUrl;
    if (!platformUrl)
      return;
    await openExternalUrl(`${platformUrl}/platform/`);
  }

  return (
    <div className="space-y-6">
      {actionError ? <p role="alert" className="text-xs text-danger">{actionError}</p> : null}
      <SettingsSection>
        <SettingsList>
          <SettingsRow
            title={t("account.futureGene")}
            description={sessionStatus === "checking"
              ? accountEmail
                ? `${accountEmail} · ${t("account.checking")}`
                : t("account.checking")
              : sessionStatus === "invalid"
                ? t("account.sessionExpired")
                : sessionStatus === "unavailable"
                  ? accountEmail
                    ? `${accountEmail} · ${t("account.temporarilyUnavailable")}`
                    : t("account.temporarilyUnavailable")
                  : loggedIn
                    ? signedInLabel
                    : t("account.loggedOut")}
          >
            {sessionStatus === "checking"
              ? null
              : sessionStatus === "invalid" || sessionStatus === "signed_out"
                ? (
                    <Button
                      onClick={() => emitFutureEvent("show-onboarding", undefined)}
                      size="sm"
                      variant="primary"
                    >
                      {t(sessionStatus === "invalid" ? "account.loginAgain" : "account.login")}
                    </Button>
                  )
                : sessionStatus === "unavailable"
                  ? confirmingLogout
                    ? (
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-ink-muted">{t("account.confirmLogout")}</span>
                          <Button onClick={() => void handleLogout()} size="sm" variant="danger">
                            {t("account.logoutConfirm")}
                          </Button>
                          <Button onClick={() => setConfirmingLogout(false)} size="sm" variant="secondary">
                            {t("account.cancel")}
                          </Button>
                        </div>
                      )
                    : (
                        <div className="flex items-center gap-2">
                          <Button onClick={onRefreshAuth} size="sm" variant="secondary">
                            {t("account.retry")}
                          </Button>
                          <Button
                            className="text-ink-soft hover:text-danger"
                            onClick={() => setConfirmingLogout(true)}
                            size="sm"
                            variant="secondary"
                          >
                            {t("account.logout")}
                          </Button>
                        </div>
                      )
                  : confirmingLogout
                    ? (
                        <div className="flex items-center gap-2">
                          <span className="text-xs text-ink-muted">{t("account.confirmLogout")}</span>
                          <Button onClick={() => void handleLogout()} size="sm" variant="danger">
                            {t("account.logoutConfirm")}
                          </Button>
                          <Button onClick={() => setConfirmingLogout(false)} size="sm" variant="secondary">
                            {t("account.cancel")}
                          </Button>
                        </div>
                      )
                    : (
                        <div className="flex items-center gap-2">
                          <Button
                            disabled={!environment.data}
                            onClick={() => void handleOpenAccount()}
                            size="sm"
                            variant="secondary"
                          >
                            {t("account.viewInfo")}
                          </Button>
                          <Button
                            className="text-ink-soft hover:text-danger"
                            onClick={() => setConfirmingLogout(true)}
                            size="sm"
                            variant="secondary"
                          >
                            {t("account.logout")}
                          </Button>
                        </div>
                      )}
          </SettingsRow>
          {loggedIn && !communityEdition
            ? (
                <SettingsRow
                  title={t("account.balance")}
                  description={balanceStatus === "loading"
                    ? t("account.balanceLoading")
                    : balanceStatus === "unavailable"
                      ? t("account.balanceUnavailable")
                      : balance != null
                        ? `${Math.trunc(balance)} ${t("account.credits")}`
                        : "—"}
                >
                  <Button
                    disabled={balanceStatus !== "unavailable" && !platformUrl}
                    onClick={balanceStatus === "unavailable" ? onRefreshBalance : () => void handleRecharge()}
                    size="sm"
                    variant="primary"
                  >
                    {t(balanceStatus === "unavailable" ? "account.retry" : "account.recharge")}
                  </Button>
                </SettingsRow>
              )
            : null}
        </SettingsList>
      </SettingsSection>
    </div>
  );
}
