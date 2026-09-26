import { AppState, Platform } from "react-native";
import { AppAlert as Alert } from "../../components/appAlerts";
import * as Notifications from "expo-notifications";
import { notifyTaskFinished, prepareTaskNotifications } from "../taskNotifications";

jest.mock("../../i18n", () => ({ __esModule: true, default: { t: (key: string) => key } }));
jest.mock("expo-notifications", () => ({
  setNotificationHandler: jest.fn(),
  setNotificationChannelAsync: jest.fn().mockResolvedValue(null),
  getPermissionsAsync: jest.fn(),
  requestPermissionsAsync: jest.fn().mockResolvedValue({ granted: true }),
  scheduleNotificationAsync: jest.fn().mockResolvedValue("notification"),
  AndroidImportance: { HIGH: 4 },
  IosAuthorizationStatus: { PROVISIONAL: 3 },
}));
const session = { sessionId: "s1", threadId: "t1", title: "My task", streaming: false, status: "completed" };
const permission = jest.mocked(Notifications.getPermissionsAsync);
const allow = (granted: boolean) => permission.mockResolvedValue({ granted, canAskAgain: false } as Awaited<ReturnType<typeof Notifications.getPermissionsAsync>>);
const originalOS = Platform.OS;
beforeEach(() => {
  jest.clearAllMocks();
  AppState.currentState = "active";
  allow(true);
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
});
afterEach(() => {
  Object.defineProperty(Platform, "OS", { configurable: true, value: originalOS });
  jest.restoreAllMocks();
});

test("Android creates a high-importance channel before requesting permission, coalescing preparation", async () => {
  Object.defineProperty(Platform, "OS", { configurable: true, value: "android" });
  permission.mockResolvedValue({ granted: false, canAskAgain: true } as Awaited<ReturnType<typeof Notifications.getPermissionsAsync>>);
  const first = prepareTaskNotifications();
  expect(prepareTaskNotifications()).toBe(first);
  await first;
  expect(Notifications.setNotificationChannelAsync).toHaveBeenCalledWith("task-completion", expect.objectContaining({ importance: 4 }));
  expect(Notifications.requestPermissionsAsync).toHaveBeenCalledTimes(1);
});

test.each(["android", "ios"])("%s delivers a local completion notification", async os => {
  Object.defineProperty(Platform, "OS", { configurable: true, value: os });
  await notifyTaskFinished(session, () => true);
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledWith({
    content: { title: "notifications.completed", body: "My task", sound: "default" },
    trigger: os === "android" ? { channelId: "task-completion" } : null,
  });
  expect(Alert.alert).not.toHaveBeenCalled();
});

test("permission denial falls back to a foreground alert, never a hidden background dialog", async () => {
  allow(false);
  await notifyTaskFinished({ ...session, status: "failed" }, () => true);
  expect(Alert.alert).toHaveBeenCalledWith("notifications.failed", "My task");
  AppState.currentState = "background";
  await notifyTaskFinished(session, () => true);
  expect(Alert.alert).toHaveBeenCalledTimes(1);
  expect(Notifications.scheduleNotificationAsync).not.toHaveBeenCalled();
});

test("native delivery failure falls back without breaking sync; switched pairings do not notify", async () => {
  jest.mocked(Notifications.scheduleNotificationAsync).mockRejectedValueOnce(new Error("native unavailable"));
  await notifyTaskFinished(session, () => true);
  expect(Alert.alert).toHaveBeenCalledTimes(1);
  jest.clearAllMocks();
  await notifyTaskFinished(session, () => false);
  expect(Alert.alert).not.toHaveBeenCalled();
  expect(Notifications.scheduleNotificationAsync).not.toHaveBeenCalled();
});

/** Re-import the module against the current mocks, returning its registration. */
function freshModule() {
  jest.resetModules();
  const notif = jest.requireMock("expo-notifications") as {
    getPermissionsAsync: jest.Mock;
    setNotificationHandler: jest.Mock;
  };
  // eslint-disable-next-line @typescript-eslint/no-require-imports -- fresh module instance
  const module = require("../taskNotifications") as typeof import("../taskNotifications");
  return { handler: notif.setNotificationHandler.mock.calls[0]?.[0], module, notif };
}

test("a foreground notification is shown as a banner, listed, and audible, but never badged", async () => {
  const { handler } = freshModule();
  expect(handler).toBeDefined();
  // These four flags are the whole product decision for a task-completion
  // notification; the app owns no unread badge, so it must stay off.
  await expect(
    (handler as { handleNotification(): Promise<Record<string, boolean>> }).handleNotification(),
  ).resolves.toEqual({
    shouldShowBanner: true,
    shouldShowList: true,
    shouldPlaySound: true,
    shouldSetBadge: false,
  });
});

test("a failed preparation is not cached, so the next launch can retry", async () => {
  const { module, notif } = freshModule();
  const permissions = notif.getPermissionsAsync as jest.Mock;
  permissions.mockRejectedValueOnce(new Error("native unavailable"));
  await expect(module.prepareTaskNotifications()).resolves.toBeUndefined();
  permissions.mockResolvedValueOnce({ granted: true, canAskAgain: false });
  // The rejection cleared the memoised promise: a second call really retries.
  await expect(module.prepareTaskNotifications()).resolves.toBeUndefined();
  expect(permissions).toHaveBeenCalledTimes(2);
  // …and a success is memoised: a third call reuses the same promise.
  await expect(module.prepareTaskNotifications()).resolves.toBeUndefined();
  expect(permissions).toHaveBeenCalledTimes(2);
});

