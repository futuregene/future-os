import type { ApprovalTier } from "../../integrations/storage/appSettings";
import { useTranslation } from "react-i18next";
import { Select } from "../../components/ui/Select";
import { automaticApprovalAvailable, effectiveApprovalTier } from "../../integrations/agent/automaticApproval";
import { isWriteOnlySandbox, useSandboxAvailability } from "../../integrations/agent/useSandboxAvailability";
import { isLinux, isWindows } from "../../lib/platform";
import { linuxUnavailableReasonKey } from "./linuxSandboxStatus";
import { SettingsList, SettingsRow, SettingsSection } from "./SettingsPrimitives";

/**
 * What the agent may run on this machine, and the diagnostics behind the
 * options that are unavailable here. Kept apart from the conversation pages
 * because it governs every run rather than one kind of reply.
 */
export function PermissionPage({
  approvalTier,
  futureSessionStatus = "checking",
  onChangeApprovalTier,
}: {
  approvalTier: ApprovalTier;
  futureSessionStatus?: string;
  onChangeApprovalTier: (value: ApprovalTier) => void;
}) {
  const { t } = useTranslation("settings");
  const sandboxAvailability = useSandboxAvailability();
  const writeOnlySandbox = isWriteOnlySandbox(sandboxAvailability);
  const autoAvailable = automaticApprovalAvailable(futureSessionStatus);
  const visibleApprovalTier = effectiveApprovalTier(approvalTier, futureSessionStatus);

  return (
    <div className="space-y-6">
      <SettingsSection>
        <SettingsList>
          <SettingsRow
            title={t("approvalTier.title")}
            description={t(
              visibleApprovalTier === "sandbox" && isWindows
                ? "approvalTier.description.sandboxWindows"
                : visibleApprovalTier === "sandbox" && isLinux
                  ? "approvalTier.description.sandboxLinux"
                  : `approvalTier.description.${visibleApprovalTier}`,
            )}
          >
            <Select
              size="sm"
              value={visibleApprovalTier}
              wrapperClassName="w-40"
              onChange={e => onChangeApprovalTier(e.target.value as ApprovalTier)}
            >
              <option value="manual">{t("approvalTier.manual")}</option>
              <option disabled={!sandboxAvailability.available} value="sandbox">
                {t(sandboxAvailability.available
                  ? "approvalTier.sandbox"
                  : sandboxAvailability.resolved
                    ? "approvalTier.sandboxUnavailable"
                    : "approvalTier.sandboxChecking")}
              </option>
              <option disabled={!sandboxAvailability.available || !autoAvailable} value="auto">
                {t(autoAvailable ? "approvalTier.auto" : futureSessionStatus === "checking" ? "approvalTier.autoChecking" : "approvalTier.autoSignInRequired")}
              </option>
              <option value="off">{t("approvalTier.off")}</option>
            </Select>
          </SettingsRow>
          {visibleApprovalTier === "auto" && sandboxAvailability.resolved && !sandboxAvailability.available
            ? <SettingsRow title={t("approvalTier.auto")} description={t("approvalTier.autoUnavailable")} />
            : null}
          {writeOnlySandbox
            ? <SettingsRow title={t("approvalTier.sandbox")} description={t("approvalTier.linuxWriteOnly")} />
            : null}
          {isLinux && sandboxAvailability.resolved && !sandboxAvailability.available
            ? (
                <SettingsRow
                  title={t("approvalTier.linuxUnavailable.title")}
                  description={t(`approvalTier.linuxUnavailable.reasons.${linuxUnavailableReasonKey(sandboxAvailability.code)}`, {
                    code: sandboxAvailability.code ?? "probe_failed",
                  })}
                />
              )
            : null}
        </SettingsList>
      </SettingsSection>
    </div>
  );
}
