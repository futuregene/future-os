import type { Language } from "../../i18n";
import { useTranslation } from "react-i18next";
import { Select } from "../../components/ui/Select";
import { getLanguage, LANGUAGE_LABELS, setLanguage, SUPPORTED_LANGUAGES } from "../../i18n";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

export function GeneralPage({
  autoApprove,
  onToggleAutoApprove,
  showThinking,
  onToggleShowThinking,
}: {
  autoApprove: boolean;
  onToggleAutoApprove: (value: boolean) => void;
  showThinking: boolean;
  onToggleShowThinking: (value: boolean) => void;
}) {
  const { t } = useTranslation("settings");

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
          title={t("autoApprove.title")}
          description={t("autoApprove.description")}
        >
          <Switch checked={autoApprove} label={t("autoApprove.title")} onChange={onToggleAutoApprove} />
        </SettingsRow>
        <SettingsRow
          title={t("showThinking.title")}
          description={t("showThinking.description")}
        >
          <Switch checked={showThinking} label={t("showThinking.title")} onChange={onToggleShowThinking} />
        </SettingsRow>
      </SettingsList>
    </SettingsSection>
  );
}
