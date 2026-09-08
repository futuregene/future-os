import type { FutureEnvironment, ProvidersView } from "../../integrations/agent/providers";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { getFutureEnvironment, listAgentProviders, logoutFutureProvider, peekAgentProviders } from "../../integrations/agent/providers";
import { openExternalUrl } from "../../integrations/storage/files";
import { emitFutureEvent } from "../../lib/futureEvents";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

/**
 * Account page. Login state is FutureGene provider login — the same signal the
 * Providers page uses (`future` builtin's `hasApiKey`). Signed out: a login
 * button that shows the onboarding gate (same guided flow as first launch).
 * Signed in: open the account page (platform URL follows the current
 * environment) plus sign out.
 */
export function AccountPage({
  balance,
  communityEdition,
  email: accountEmail,
  onRefreshBalance,
}: {
  balance: number | null;
  communityEdition: boolean;
  email: string | null;
  onRefreshBalance: () => void;
}) {
  const { t } = useTranslation("settings");
  const { data: providers, loading, reload } = useAsyncResource<ProvidersView | null>(
    listAgentProviders,
    [],
    peekAgentProviders(),
  );
  // The platform host follows the active environment (test vs production).
  const environment = useAsyncResource<FutureEnvironment | null>(
    getFutureEnvironment,
    [],
    null,
  );
  const [confirmingLogout, setConfirmingLogout] = useState(false);

  const loggedIn = Boolean(providers?.builtin.find(provider => provider.id === "future")?.hasApiKey);

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
    // logoutFutureProvider clears the profile cache internally.
    await logoutFutureProvider();
    setConfirmingLogout(false);
    reload();
  }

  async function handleOpenAccount() {
    const platformUrl = environment.data?.platformUrl;
    if (!platformUrl)
      return;
    await openExternalUrl(`${platformUrl}/platform/`);
  }

  if (loading && !providers) {
    return null;
  }

  return (
    <div className="space-y-6">
      <SettingsSection>
        <SettingsList>
          <SettingsRow
            title={t("account.futureGene")}
            description={loggedIn ? signedInLabel : t("account.loggedOut")}
          >
            {!loggedIn
              ? (
                  <Button
                    onClick={() => emitFutureEvent("show-onboarding", undefined)}
                    size="sm"
                    variant="primary"
                  >
                    {t("account.login")}
                  </Button>
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
                  description={balance != null ? `${Math.trunc(balance)} ${t("account.credits")}` : "—"}
                >
                  <Button
                    disabled={!platformUrl}
                    onClick={() => void handleRecharge()}
                    size="sm"
                    variant="primary"
                  >
                    {t("account.recharge")}
                  </Button>
                </SettingsRow>
              )
            : null}
        </SettingsList>
      </SettingsSection>
    </div>
  );
}
