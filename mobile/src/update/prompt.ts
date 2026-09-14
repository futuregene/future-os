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
export function promptUpgrade(status: UpdateStatus, t: TFunction): void {
  const appStore = Platform.OS === "ios";
  Alert.alert(
    t("update.title"),
    t(appStore ? "update.appStoreMessage" : status.canInstallInApp ? "update.message" : "update.manualMessage", {
      current: displayVersion(status.currentVersion, t),
      version: displayVersion(status.latestVersion, t),
    }),
    [
      { text: t("update.cancel"), style: "cancel" },
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
