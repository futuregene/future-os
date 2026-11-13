import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import * as Clipboard from "expo-clipboard";
import { useAppDialog } from "../../components/useAppDialog";
import { shortDeviceId } from "../../remote/deviceId";
import { loadDeviceId } from "../../remote/storage";
import { SettingsLink } from "./SettingsPrimitives";

/**
 * "This device's ID", on the phone's own settings.
 *
 * The id is minted when this phone is first paired and kept in the system
 * keychain, so the row exists only once there is one: before a pairing the
 * phone has no identity to show, and minting one here would hand out a value
 * the platform has never seen.
 *
 * The row prints the short form — the same `dev_…` prefix and eight characters
 * the paired machine's device list prints — and the tap opens the full value so
 * it can be copied. The short form is what a person compares between the two
 * screens; the full one is only ever needed to hand to someone else.
 *
 * A keychain that cannot be read is not a reason to fail the settings page: the
 * row stays absent, exactly as on a phone that was never paired.
 */
export function ThisDeviceIdRow() {
  const { t } = useTranslation();
  const { alert, dialog } = useAppDialog();
  const [deviceId, setDeviceId] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    void loadDeviceId()
      .then(id => { if (live) setDeviceId(id); })
      .catch(() => {});
    return () => { live = false; };
  }, []);
  if (!deviceId) return null;
  return <>
    <SettingsLink
      description={shortDeviceId(deviceId)}
      label={t("desktopSettings.thisDeviceId")}
      onPress={() => alert(t("desktopSettings.thisDeviceId"), deviceId, [
        { text: t("common.close"), style: "cancel" },
        {
          text: t("desktopSettings.thisDeviceIdCopy"),
          // The dialog dismisses first and this runs after it (see
          // `useAppDialog`), so a copy that fails cannot reopen a closed
          // surface. A clipboard refusal is silent: the id is on screen either
          // way, which is the half the user came for.
          onPress: () => { void Clipboard.setStringAsync(deviceId).catch(() => {}); },
        },
      ])}
    />
    {dialog}
  </>;
}
