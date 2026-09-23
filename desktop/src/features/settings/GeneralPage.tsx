import type { Language } from "../../i18n";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import { useTranslation } from "react-i18next";
import { Select } from "../../components/ui/Select";
import { getLanguage, LANGUAGE_LABELS, setLanguage, SUPPORTED_LANGUAGES } from "../../i18n";
import { useSandboxAvailability } from "../../integrations/agent/useSandboxAvailability";
import { isLinux, isWindows } from "../../lib/platform";
import { linuxUnavailableReasonKey } from "./linuxSandboxStatus";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

export function GeneralPage({
  approvalTier,
  onChangeApprovalTier,
  autoUpgradeSkills,
  onToggleAutoUpgradeSkills,
  skillRecommend,
  onToggleSkillRecommend,
  bellOnComplete,
  onToggleBellOnComplete,
  autoTitleFirstTurn,
  onToggleAutoTitleFirstTurn,
}: {
  approvalTier: ApprovalTier;
  onChangeApprovalTier: (value: ApprovalTier) => void;
  autoUpgradeSkills: boolean;
  onToggleAutoUpgradeSkills: (value: boolean) => void;
  skillRecommend: boolean;
  onToggleSkillRecommend: (value: boolean) => void;
  bellOnComplete: boolean;
  onToggleBellOnComplete: (value: boolean) => void;
  autoTitleFirstTurn: boolean;
  onToggleAutoTitleFirstTurn: (value: boolean) => void;
}) {
  const { t } = useTranslation("settings");
  const sandboxAvailability = useSandboxAvailability();

  return (
    <SettingsSection>
      <SettingsList>
        <SettingsRow
          title={t("language.title")}
          description={t("language.description")}
        >
          <Select
            size="sm"
            value={getLanguage()}
            wrapperClassName="w-32"
            onChange={e => setLanguage(e.target.value as Language)}
          >
            {SUPPORTED_LANGUAGES.map(lang => (
              <option key={lang} value={lang}>
                {LANGUAGE_LABELS[lang]}
              </option>
            ))}
          </Select>
        </SettingsRow>
        <SettingsRow
          title={t("approvalTier.title")}
          description={t(
            approvalTier === "sandbox" && isWindows
              ? "approvalTier.description.sandboxWindows"
              : approvalTier === "sandbox" && isLinux
                ? "approvalTier.description.sandboxLinux"
                : `approvalTier.description.${approvalTier}`,
          )}
        >
          <Select
            size="sm"
            value={approvalTier}
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
            <option value="off">{t("approvalTier.off")}</option>
          </Select>
        </SettingsRow>
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
        <SettingsRow
          title={t("autoUpgradeSkills.title")}
          description={t("autoUpgradeSkills.description")}
        >
          <Switch checked={autoUpgradeSkills} label={t("autoUpgradeSkills.title")} onChange={onToggleAutoUpgradeSkills} />
        </SettingsRow>
        <SettingsRow
          title={t("skillRecommend.title")}
          description={t("skillRecommend.description")}
        >
          <Switch checked={skillRecommend} label={t("skillRecommend.title")} onChange={onToggleSkillRecommend} />
        </SettingsRow>
        <SettingsRow
          title={t("autoTitleFirstTurn.title")}
          description={t("autoTitleFirstTurn.description")}
        >
          <Switch checked={autoTitleFirstTurn} label={t("autoTitleFirstTurn.title")} onChange={onToggleAutoTitleFirstTurn} />
        </SettingsRow>
        <SettingsRow
          title={t("bellOnComplete.title")}
          description={t("bellOnComplete.description")}
        >
          <Switch checked={bellOnComplete} label={t("bellOnComplete.title")} onChange={onToggleBellOnComplete} />
        </SettingsRow>
      </SettingsList>
    </SettingsSection>
  );
}
