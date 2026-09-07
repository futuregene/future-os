import { Alert } from "react-native";
import type { TFunction } from "i18next";
import { installUpdate, type UpdateStatus } from "./update";

/** Upgrade dialog — confirm downloads/installs (iOS opens the App Store). */
export function promptUpgrade(status: UpdateStatus, t: TFunction): void {
  Alert.alert(
    t("update.title"),
    t(status.canInstallInApp ? "update.message" : "update.manualMessage", {
      current: status.currentVersion,
      version: status.latestVersion,
    }),
    [
      { text: t("update.cancel"), style: "cancel" },
      {
        text: t(status.canInstallInApp ? "update.confirm" : "update.download"),
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
