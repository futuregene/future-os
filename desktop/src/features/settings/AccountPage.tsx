import type { ReactNode } from "react";
import type { FutureBalanceStatus, FutureSessionStatus } from "../../components/layout/hooks/useFutureAccount";
import type { FutureEnvironment } from "../../integrations/agent/providers";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { getFutureEnvironment, logoutFutureProvider } from "../../integrations/agent/providers";
import { openExternalUrl } from "../../integrations/storage/files";
import { cn } from "../../lib/cn";
import { formatDateTime } from "../../lib/date";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";
import { formatCredits, PLAN_STATUS_KEYS, windowUsagePercent } from "./subscriptionPresentation";
import { useFutureTokenPlan } from "./useFutureTokenPlan";

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
  const { t, i18n } = useTranslation("settings");
  // The platform host follows the active environment (test vs production).
  const environment = useAsyncResource<FutureEnvironment | null>(
    getFutureEnvironment,
    [],
    null,
  );
  const [confirmingLogout, setConfirmingLogout] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const loggedIn = sessionStatus === "authenticated" || sessionStatus === "unavailable";
  // The subscription is platform-billed, so the community edition (own models
  // and keys) has none to show — the same rule the balance row follows.
  const subscription = useFutureTokenPlan(loggedIn && !communityEdition);

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

  // What the balance row says. A bare number leaves the reader guessing whether
  // zero is normal, so a zero balance says what it means and every state the
  // account store can be in renders distinctly (the em dash this replaced said
  // nothing at all about "not read yet").
  const balanceText = balanceStatus === "loading"
    ? t("account.balanceLoading")
    : balanceStatus === "unavailable"
      ? t("account.balanceUnavailable")
      : balance == null
        ? t("account.balanceUnknown")
        : balance <= 0
          ? `${Math.trunc(balance)} ${t("account.credits")} · ${t("account.balanceEmpty")}`
          : `${Math.trunc(balance)} ${t("account.credits")}`;
  const showsBalance = loggedIn && !communityEdition;
  // Credits are a platform concept the page never explained: what they are,
  // what a zero balance stops, and why the community edition hides the row.
  const balanceHint = communityEdition
    ? t("account.balanceCommunity")
    : showsBalance
      ? t("account.balanceHint")
      : null;

  async function handleRecharge() {
    if (!platformUrl)
      return;
    await openExternalUrl(`${platformUrl}/platform/#recharge`);
  }

  // Both subscription actions land on the site's public subscription page.
  // The console's `#plan` view needs a web login of its own — a desktop session
  // does not carry over to the external browser — so deep-linking there opened
  // the marketing home instead. `/plans` renders for everyone and shows the
  // current tier (and its renewal) to a signed-in visitor.
  async function handleOpenSubscriptionPage() {
    if (!platformUrl)
      return;
    try {
      await openExternalUrl(`${platformUrl}/plans`);
    }
    catch (error) {
      setActionError(errorMessage(error));
    }
  }

  // Subscription rows: tier, end date and the current window's usage. The
  // platform reports "no subscription" as an empty, inactive snapshot rather
  // than an error, so an account that never subscribed still gets a row that
  // says so and a way to the site's subscription page.
  const plan = subscription.tokenPlan;
  const planName = plan?.name || plan?.planCode || "";
  const hasSubscription = Boolean(plan?.planCode);
  const statusLabel = plan?.status ? t(PLAN_STATUS_KEYS[plan.status]) : null;
  const grantedCredits = plan?.grantedCredits ?? 0;
  const usedCredits = plan?.usedCredits ?? 0;
  const remainingCredits = Math.max(0, plan?.remainingCredits ?? 0);
  const overrun = usedCredits > grantedCredits;
  const usagePercent = windowUsagePercent(grantedCredits, usedCredits);
  const showsWindow = Boolean(plan?.active && plan.windowEnd);
  const windowResetAt = plan?.windowResetsAt ?? plan?.windowEnd;
  let windowDescription: string | null = null;
  if (plan) {
    if (!plan.active) {
      windowDescription = plan.status === "scheduled"
        ? t("account.windowScheduled")
        : t("account.windowExpired");
    }
    else if (!plan.windowEnd) {
      windowDescription = t("account.windowNone");
    }
    else {
      windowDescription = [
        t("account.windowUsage", {
          granted: formatCredits(grantedCredits),
          used: formatCredits(usedCredits),
        }),
        t("account.windowRemaining", { remaining: formatCredits(remainingCredits) }),
        windowResetAt
          ? t("account.windowReset", { time: formatDateTime(windowResetAt, i18n.language) })
          : null,
        overrun ? t("account.windowOverrun") : null,
      ].filter(Boolean).join(" · ");
    }
  }

  let subscriptionRows: ReactNode;
  if (plan && hasSubscription) {
    subscriptionRows = (
      <>
        <SettingsRow
          title={t("account.plan")}
          description={statusLabel ? `${planName} · ${statusLabel}` : planName}
        >
          <Button
            disabled={!platformUrl}
            onClick={() => void handleOpenSubscriptionPage()}
            size="sm"
            variant="secondary"
          >
            {t("account.planView")}
          </Button>
        </SettingsRow>
        {plan.endsAt
          ? (
              <SettingsRow
                title={t("account.planEndsAt")}
                description={formatDateTime(plan.endsAt, i18n.language)}
              />
            )
          : null}
        <SettingsRow
          description={windowDescription}
          stacked
          title={t("account.windowQuota")}
        >
          {showsWindow
            ? (
                <div
                  aria-label={t("account.windowQuota")}
                  aria-valuemax={100}
                  aria-valuemin={0}
                  aria-valuenow={Math.round(usagePercent)}
                  className="h-1.5 w-full overflow-hidden rounded-full bg-line-soft"
                  role="progressbar"
                >
                  <div
                    className={cn("h-full rounded-full transition-[width]", overrun ? "bg-danger" : "bg-accent")}
                    style={{ width: `${usagePercent}%` }}
                  />
                </div>
              )
            : null}
        </SettingsRow>
      </>
    );
  }
  else if (subscription.status === "unavailable") {
    subscriptionRows = (
      <SettingsRow
        description={t("account.subscriptionUnavailable")}
        title={t("account.plan")}
      >
        <Button onClick={subscription.refresh} size="sm" variant="secondary">
          {t("account.retry")}
        </Button>
      </SettingsRow>
    );
  }
  else if (subscription.status === "available") {
    subscriptionRows = (
      <SettingsRow description={t("account.planNone")} title={t("account.plan")}>
        <Button
          disabled={!platformUrl}
          onClick={() => void handleOpenSubscriptionPage()}
          size="sm"
          variant="primary"
        >
          {t("account.viewPlans")}
        </Button>
      </SettingsRow>
    );
  }
  else {
    subscriptionRows = <SettingsRow description={t("account.subscriptionLoading")} title={t("account.plan")} />;
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
          {showsBalance
            ? (
                <SettingsRow
                  title={t("account.balance")}
                  description={balanceText}
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
        {balanceHint ? <p className="text-xs leading-5 text-ink-muted">{balanceHint}</p> : null}
      </SettingsSection>
      {showsBalance
        ? (
            <SettingsSection title={t("account.subscription")}>
              <SettingsList>{subscriptionRows}</SettingsList>
              {subscription.status === "available" && !hasSubscription
                ? <p className="text-xs leading-5 text-ink-muted">{t("account.planNoneHint")}</p>
                : null}
            </SettingsSection>
          )
        : null}
    </div>
  );
}
