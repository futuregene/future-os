import type { FutureProfile, ProvidersView } from "../../integrations/agent/providers";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { getFutureProfile, listAgentProviders, logoutFutureProvider, peekFutureProfile } from "../../integrations/agent/providers";
import { openExternalUrl } from "../../integrations/storage/files";
import { invokeCommand } from "../../integrations/tauri/invoke";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { FutureLoginDialog } from "./FutureLoginDialog";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

interface FutureEnvironment {
  /** `production` | `test` | `custom`. */
  environment: string;
  /** Resolved platform root currently in effect (no `/api`). */
  platformUrl: string;
}

/**
 * Account page. Login state is FutureGene provider login — the same signal the
 * Providers page uses (`future` builtin's `hasApiKey`). Signed out: only a login
 * button. Signed in: open the account page (platform URL follows the current
 * environment) plus sign out.
 */
export function AccountPage() {
  const { t } = useTranslation("settings");
  const { data: providers, loading, reload } = useAsyncResource<ProvidersView | null>(
    listAgentProviders,
    [],
    null,
  );
  // The platform host follows the active environment (test vs production).
  const environment = useAsyncResource<FutureEnvironment | null>(
    () => invokeCommand<FutureEnvironment>("get_future_environment"),
    [],
    null,
  );
  const [loginOpen, setLoginOpen] = useState(false);
  const [confirmingLogout, setConfirmingLogout] = useState(false);

  const loggedIn = Boolean(providers?.builtin.find(provider => provider.id === "future")?.hasApiKey);

  // Show the signed-in email (like `future account profile`) instead of a bare
  // "signed in" label. Seeded synchronously from the session cache so reopening
  // this page doesn't flash — the fetch only really runs the first time (or
  // after a logout clears the cache). Falls back to the generic label until it
  // resolves, or if the request fails.
  const profile = useAsyncResource<FutureProfile | null>(
    () => (loggedIn ? getFutureProfile() : Promise.resolve(null)),
    [loggedIn],
    peekFutureProfile(),
  );
  const signedInLabel = profile.data?.email ?? t("account.loggedIn");

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

  if (loading) {
    return <p className="text-sm text-ink-muted">{t("account.loading")}</p>;
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
                  <Button onClick={() => setLoginOpen(true)} size="sm" variant="primary">
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
        </SettingsList>
      </SettingsSection>

      <FutureLoginDialog
        onAuthorized={() => {
          // pollFutureLogin already cleared the profile cache on "authorized",
          // so the reload below refetches fresh once `loggedIn` flips true.
          setLoginOpen(false);
          reload();
        }}
        onClose={() => setLoginOpen(false)}
        open={loginOpen}
      />
    </div>
  );
}
