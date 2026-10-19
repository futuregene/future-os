import { useTranslation } from "react-i18next";
import { Switch } from "../../components/ui/Switch";
import { openExternalUrl } from "../../integrations/storage/files";
import { SettingsPage, SettingsRow, SettingsSection } from "../settings/SettingsPrimitives";

/**
 * This direction's own settings page.
 *
 * Configuration, not list content: the hub shows the paired clients, and the
 * one preference that belongs to *this* direction of remote (whether to
 * reconnect them when the app opens) lives behind the settings control rather
 * than under the list. Keeping it here also keeps the off state honest — with
 * the remote desktop switched off there is nothing to configure, so the page is
 * unreachable.
 */
export function RemoteHostSettingsPage({
  autoConnect,
  onBack,
  onToggleAutoConnect,
}: {
  autoConnect: boolean;
  onBack: () => void;
  onToggleAutoConnect: (value: boolean) => void;
}) {
  const { t } = useTranslation("remoteHub");
  const { t: tRemote } = useTranslation("remote");

  return (
    <SettingsPage
      backLabel={tRemote("back")}
      onBack={onBack}
      title={t("hostSettingsTitle")}
    >
      <SettingsSection>
        <div className="rounded-lg border border-line-soft bg-surface px-4">
          <SettingsRow
            description={t("autoConnectDescription")}
            title={t("autoConnectTitle")}
          >
            <Switch
              checked={autoConnect}
              label={t("autoConnectTitle")}
              onChange={onToggleAutoConnect}
            />
          </SettingsRow>
        </div>
      </SettingsSection>

      <p className="text-xs text-ink-muted">
        {tRemote("note")}
        {" "}
        <button
          className="text-accent hover:underline"
          onClick={() => void openExternalUrl("https://future-os.cn/#download-mobile")}
          type="button"
        >
          {tRemote("downloadMobile")}
        </button>
      </p>
    </SettingsPage>
  );
}
