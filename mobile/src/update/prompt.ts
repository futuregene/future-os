import { Platform } from "react-native";
import { AppAlert as Alert } from "../components/appAlerts";
import type { TFunction } from "i18next";
import { buildChannel, installUpdate, type UpdateStatus } from "./update";

function displayVersion(version: string, t: TFunction): string {
  const channel = buildChannel(version);
  if (channel === "release") return version;
  const match = /^(\d+\.\d+\.\d+)-([a-z0-9]+)(?:\+(test|nightly|dev|local(?:\.dirty)?))?$/.exec(version);
  if (!match) return version;
  const label = t(`update.channels.${match[3] === "local.dirty" ? "localDirty" : channel}`);
  return `${match[1]} · ${label} (${match[2]})`;
}

/** Upgrade dialog — confirm downloads/installs (iOS opens the App Store). */
export function promptUpgrade(
  status: UpdateStatus,
  t: TFunction,
  /**
   * Called when the user answers "later" (or dismisses the dialog), so a caller
   * that reminds on its own schedule can remember not to ask again. Reached
   * through the cancel button's onPress, which the dialog host also runs for a
   * backdrop or back-button dismissal.
   */
  onDismiss?: () => void,
): void {
  const appStore = Platform.OS === "ios";
  Alert.alert(
    t("update.title"),
    t(appStore ? "update.appStoreMessage" : status.canInstallInApp ? "update.message" : "update.manualMessage", {
      current: displayVersion(status.currentVersion, t),
      version: displayVersion(status.latestVersion, t),
    }),
    [
      { text: t("update.cancel"), style: "cancel", onPress: onDismiss },
      {
        text: t(appStore ? "update.appStore" : status.canInstallInApp ? "update.confirm" : "update.download"),
        onPress: () => {
          void installUpdate(status).catch(() => {
            Alert.alert(t("update.title"), t("update.installFailed"));
          });
        },
      },
    ],
    { cancelable: true },
  );
}
