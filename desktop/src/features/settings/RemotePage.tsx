import { useTranslation } from "react-i18next";
import { SettingsList, SettingsRow, SettingsSection, Switch } from "./SettingsPrimitives";

/**
 * Remote-control preferences. The Remote feature (and thus this page) is only
 * reachable on non-release builds — the nav item is gated `devOnly` in
 * `SettingsDialog`. Kept as its own page so the (single, today) toggle has room
 * to grow without crowding General.
 */
export function RemotePage({
  autoConnectRemote,
  onToggleAutoConnectRemote,
}: {
  autoConnectRemote: boolean;
  onToggleAutoConnectRemote: (value: boolean) => void;
}) {
  const { t } = useTranslation("settings");

  return (
    <SettingsSection>
      <SettingsList>
        <SettingsRow
          title={t("remote.autoConnect.title")}
          description={t("remote.autoConnect.description")}
        >
          <Switch
            checked={autoConnectRemote}
            label={t("remote.autoConnect.title")}
            onChange={onToggleAutoConnectRemote}
          />
        </SettingsRow>
      </SettingsList>
    </SettingsSection>
  );
}
