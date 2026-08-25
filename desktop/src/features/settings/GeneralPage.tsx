import type { Language } from "../../i18n";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import { useTranslation } from "react-i18next";
import { Select } from "../../components/ui/Select";
import { getLanguage, LANGUAGE_LABELS, setLanguage, SUPPORTED_LANGUAGES } from "../../i18n";
import { useSandboxAvailability } from "../../integrations/agent/useSandboxAvailability";
import { isWindows } from "../../lib/platform";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

export function GeneralPage({
  approvalTier,
  onChangeApprovalTier,
  showThinking,
  onToggleShowThinking,
  autoUpgradeSkills,
  onToggleAutoUpgradeSkills,
}: {
  approvalTier: ApprovalTier;
  onChangeApprovalTier: (value: ApprovalTier) => void;
  showThinking: boolean;
  onToggleShowThinking: (value: boolean) => void;
  autoUpgradeSkills: boolean;
  onToggleAutoUpgradeSkills: (value: boolean) => void;
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
            {sandboxAvailability.available || (approvalTier === "sandbox" && !sandboxAvailability.definitive)
              ? (
                  <option disabled={!sandboxAvailability.available} value="sandbox">
                    {t("approvalTier.sandbox")}
                  </option>
                )
              : null}
            <option value="off">{t("approvalTier.off")}</option>
          </Select>
        </SettingsRow>
        <SettingsRow
          title={t("showThinking.title")}
          description={t("showThinking.description")}
        >
          <Switch checked={showThinking} label={t("showThinking.title")} onChange={onToggleShowThinking} />
        </SettingsRow>
        <SettingsRow
          title={t("autoUpgradeSkills.title")}
          description={t("autoUpgradeSkills.description")}
        >
          <Switch checked={autoUpgradeSkills} label={t("autoUpgradeSkills.title")} onChange={onToggleAutoUpgradeSkills} />
        </SettingsRow>
      </SettingsList>
    </SettingsSection>
  );
}
