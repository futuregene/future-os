import type { AppSettings } from "../../integrations/storage/appSettings";
import { useTranslation } from "react-i18next";
import { TextInput } from "../../components/ui/TextInput";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

export function ArchiveSettingsPage({
  appSettings,
  onChangeSettings,
}: {
  appSettings: AppSettings;
  onChangeSettings: (patch: Partial<AppSettings>) => Promise<void>;
}) {
  const { t } = useTranslation("settings");
  return (
    <div className="space-y-6">
      <SettingsSection title={t("archive.cleanupSection")}>
        <SettingsList>
          <SettingsRow
            title={t("archive.cleanupEnabled")}
            description={t("archive.cleanupDescription")}
          >
            <Switch
              checked={appSettings.archiveRunLogCleanup}
              label={t("archive.cleanupEnabled")}
              onChange={value => void onChangeSettings({ archiveRunLogCleanup: value })}
            />
          </SettingsRow>
        </SettingsList>
      </SettingsSection>

      <SettingsSection
        description={t("archive.autoSectionDescription")}
        title={t("archive.autoSection")}
      >
        <SettingsList>
          <SettingsRow
            title={t("archive.autoEnabled")}
            description={t("archive.autoDescription")}
          >
            <Switch
              checked={appSettings.autoArchiveEnabled}
              label={t("archive.autoEnabled")}
              onChange={value => void onChangeSettings({ autoArchiveEnabled: value })}
            />
          </SettingsRow>
          <SettingsRow
            title={t("archive.idleDays")}
            description={t("archive.idleDaysDescription")}
          >
            <TextInput
              aria-label={t("archive.idleDays")}
              className="w-28"
              disabled={!appSettings.autoArchiveEnabled}
              max={365}
              min={1}
              onChange={(event) => {
                const days = Number(event.target.value);
                if (Number.isInteger(days) && days >= 1 && days <= 365)
                  void onChangeSettings({ autoArchiveIdleDays: days });
              }}
              type="number"
              value={appSettings.autoArchiveIdleDays}
            />
          </SettingsRow>
          <SettingsRow
            title={t("archive.cleanupInteraction")}
            description={t(appSettings.archiveRunLogCleanup
              ? "archive.autoCleanupOn"
              : "archive.autoCleanupOff")}
          />
        </SettingsList>
      </SettingsSection>
    </div>
  );
}
