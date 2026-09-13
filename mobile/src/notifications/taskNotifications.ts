import { Alert, AppState, Platform } from "react-native";
import * as Notifications from "expo-notifications";
import i18n from "../i18n";
import type { RemoteSession } from "../remote/types";
import { withNativePresentation } from "../remote/nativePresentation";

const CHANNEL_ID = "task-completion";
let preparation: Promise<void> | null = null;

Notifications.setNotificationHandler({
  handleNotification: async () => ({
    shouldShowBanner: true,
    shouldShowList: true,
    shouldPlaySound: true,
    shouldSetBadge: false,
  }),
});

/** Ask once the phone is paired, not while the user is trying to scan a QR code. */
export function prepareTaskNotifications(): Promise<void> {
  if (preparation) return preparation;
  preparation = (async () => {
    if (Platform.OS === "android") {
      await Notifications.setNotificationChannelAsync(CHANNEL_ID, {
        name: i18n.t("notifications.channel"),
        importance: Notifications.AndroidImportance.HIGH,
        sound: "default",
      });
    }
    const permission = await Notifications.getPermissionsAsync();
    if (!permission.granted && permission.canAskAgain && AppState.currentState === "active") {
      await withNativePresentation(() => Notifications.requestPermissionsAsync());
    }
  })().catch(() => {
    // Native notifications may be unavailable (e.g. an older development client).
    // Keep chat usable and let the next paired launch retry preparation.
    preparation = null;
  });
  return preparation;
}

/** No message content in lock-screen notifications; only the conversation title. */
export async function notifyTaskFinished(
  session: RemoteSession,
  isCurrent: () => boolean,
): Promise<void> {
  const title = i18n.t(session.status === "failed" ? "notifications.failed" : "notifications.completed");
  const body = session.title || i18n.t("sessions.unnamed");
  try {
    await prepareTaskNotifications();
    const permission = await Notifications.getPermissionsAsync();
    if (!isCurrent()) return;
    if (permission.granted || permission.ios?.status === Notifications.IosAuthorizationStatus.PROVISIONAL) {
      await Notifications.scheduleNotificationAsync({
        content: { title, body, sound: "default" },
        trigger: Platform.OS === "android" ? { channelId: CHANNEL_ID } : null,
      });
      return;
    }
  } catch {
    // Permission refusal/native delivery failure must not break remote sync.
  }
  if (isCurrent() && AppState.currentState === "active") Alert.alert(title, body);
}
