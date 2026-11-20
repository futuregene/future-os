import { useTranslation } from "react-i18next";
import { HelpTip } from "../../components/ui/HelpTip";
import { TextInput } from "../../components/ui/TextInput";
import { DatabaseOrganization } from "./DatabaseOrganization";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

/**
 * What a conversation keeps and when it is shelved: the cleanup that runs on
 * archiving, the idle timer that archives at all, and the batch pass that
 * reclaims storage across every conversation.
 */
export function StoragePage({
  archiveRunLogCleanup,
  onToggleArchiveRunLogCleanup,
  autoArchiveEnabled,
  onToggleAutoArchive,
  autoArchiveIdleDays,
  onChangeAutoArchiveIdleDays,
}: {
  archiveRunLogCleanup: boolean;
  onToggleArchiveRunLogCleanup: (value: boolean) => void;
  autoArchiveEnabled: boolean;
  onToggleAutoArchive: (value: boolean) => void;
  autoArchiveIdleDays: number;
  onChangeAutoArchiveIdleDays: (value: number) => void;
}) {
  const { t } = useTranslation("settings");

  return (
    <div className="space-y-6">
      <SettingsSection title={t("archive.autoSection")}>
        <SettingsList>
          <SettingsRow
            title={t("archive.cleanupEnabled")}
            description={t("archive.cleanupDescription")}
          >
            <Switch
              checked={archiveRunLogCleanup}
              label={t("archive.cleanupEnabled")}
              onChange={onToggleArchiveRunLogCleanup}
            />
          </SettingsRow>
          <SettingsRow
            title={t("archive.autoEnabled")}
            description={t("archive.autoDescription")}
          >
            <Switch
              checked={autoArchiveEnabled}
              label={t("archive.autoEnabled")}
              onChange={onToggleAutoArchive}
            />
          </SettingsRow>
          <SettingsRow
            title={(
              <span className="inline-flex items-center gap-1.5">
                {t("archive.idleDays")}
                <HelpTip label={t("archive.idleDaysHelpLabel")}>{t("archive.idleDaysHelp")}</HelpTip>
              </span>
            )}
            description={t("archive.idleDaysDescription")}
          >
            <TextInput
              aria-label={t("archive.idleDays")}
              className="w-28"
              disabled={!autoArchiveEnabled}
              max={365}
              min={1}
              onChange={(event) => {
                const days = Number(event.target.value);
                if (Number.isInteger(days) && days >= 1 && days <= 365)
                  onChangeAutoArchiveIdleDays(days);
              }}
              type="number"
              value={autoArchiveIdleDays}
            />
          </SettingsRow>
        </SettingsList>
      </SettingsSection>

      <DatabaseOrganization />
    </div>
  );
}
