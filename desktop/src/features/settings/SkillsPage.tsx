import { useTranslation } from "react-i18next";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

/**
 * The two skill behaviours the agent reads per run: whether installed skills are
 * upgraded when the app opens, and whether a send may suggest one skill the
 * reader does not have yet.
 */
export function SkillsPage({
  autoUpgradeSkills,
  onToggleAutoUpgradeSkills,
  skillRecommend,
  onToggleSkillRecommend,
}: {
  autoUpgradeSkills: boolean;
  onToggleAutoUpgradeSkills: (value: boolean) => void;
  skillRecommend: boolean;
  onToggleSkillRecommend: (value: boolean) => void;
}) {
  const { t } = useTranslation("settings");

  return (
    <div className="space-y-6">
      <SettingsSection>
        <SettingsList>
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
        </SettingsList>
      </SettingsSection>
    </div>
  );
}
