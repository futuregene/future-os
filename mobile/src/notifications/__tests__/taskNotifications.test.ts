import { AppState, Platform } from "react-native";
import { AppAlert as Alert } from "../../components/appAlerts";
import * as Notifications from "expo-notifications";
import { notifyTaskFinished, prepareTaskNotifications } from "../taskNotifications";
import { setTaskFinishedReminders } from "../preferences";

jest.mock("../../i18n", () => ({ __esModule: true, default: { t: jest.fn((key: string) => key) } }));
jest.mock("expo-notifications", () => ({
  setNotificationHandler: jest.fn(),
  setNotificationChannelAsync: jest.fn().mockResolvedValue(null),
  getPermissionsAsync: jest.fn(),
  requestPermissionsAsync: jest.fn().mockResolvedValue({ granted: true }),
  scheduleNotificationAsync: jest.fn().mockResolvedValue("notification"),
  addNotificationResponseReceivedListener: jest.fn(() => ({ remove: jest.fn() })),
  getLastNotificationResponse: jest.fn(() => null),
  clearLastNotificationResponse: jest.fn(),
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
  // `preparation` is memoised for the life of the module, so "the first caller"
  // is only observable on a fresh instance — which is exactly what one launch
  // gets. Sharing this file's instance would make the result depend on whether
  // an earlier test had already prepared.
  const { module: notifications, notif } = freshModule();
  // The re-import rebuilt react-native too, and the module reads the platform
  // from that instance, not from this file's import.
  // eslint-disable-next-line @typescript-eslint/no-require-imports -- the fresh instance the module above reads
  const fresh = require("react-native") as typeof import("react-native");
  const previous = fresh.Platform.OS;
  Object.defineProperty(fresh.Platform, "OS", { configurable: true, value: "android" });
  fresh.AppState.currentState = "active";
  try {
    notif.getPermissionsAsync.mockResolvedValue({ granted: false, canAskAgain: true });
    const first = notifications.prepareTaskNotifications();
    expect(notifications.prepareTaskNotifications()).toBe(first);
    await first;
    expect(notif.setNotificationChannelAsync).toHaveBeenCalledWith("task-completion", expect.objectContaining({ importance: 4 }));
    expect(notif.requestPermissionsAsync).toHaveBeenCalledTimes(1);
  } finally {
    Object.defineProperty(fresh.Platform, "OS", { configurable: true, value: previous });
  }
});

test.each(["android", "ios"])("%s delivers a local completion notification naming the machine and the conversation", async os => {
  Object.defineProperty(Platform, "OS", { configurable: true, value: os });
  await notifyTaskFinished(session, () => true, { machine: "Studio", desktopId: "desktop_1" });
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledWith({
    content: {
      title: "notifications.completed",
      body: "Studio · My task",
      sound: "default",
      // The tap target travels in the payload: it is the only thing the OS
      // hands back when the reminder is tapped.
      data: { desktopId: "desktop_1", sessionId: "s1", machine: "Studio" },
    },
    trigger: os === "android" ? { channelId: "task-completion" } : null,
  });
  expect(Alert.alert).not.toHaveBeenCalled();
});

test("a reminder from a machine the phone has no name for still says which conversation", async () => {
  await notifyTaskFinished(session, () => true, { desktopId: "desktop_1" });
  expect(Notifications.scheduleNotificationAsync).toHaveBeenCalledWith({
    content: expect.objectContaining({
      body: "My task",
      data: { desktopId: "desktop_1", sessionId: "s1", machine: "" },
    }),
    trigger: null,
  });
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

test("reminders switched off deliver nothing, not even the in-app alert", async () => {
  await setTaskFinishedReminders(false);
  try {
    // Both halves of the feature are silent: the reminder the OS would show,
    // and the alert that stands in when it may not.
    allow(false);
    await notifyTaskFinished({ ...session, status: "failed" }, () => true, { machine: "Studio" });
    expect(Notifications.scheduleNotificationAsync).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
    allow(true);
    await notifyTaskFinished(session, () => true);
    expect(Notifications.scheduleNotificationAsync).not.toHaveBeenCalled();
  } finally {
    await setTaskFinishedReminders(true);
  }
});

test("switching the reminders back on is when the OS is asked", async () => {
  const { module, notif } = freshModule();
  // eslint-disable-next-line @typescript-eslint/no-require-imports -- the instances the module above reads
  const fresh = require("react-native") as typeof import("react-native");
  Object.defineProperty(fresh.Platform, "OS", { configurable: true, value: "android" });
  // eslint-disable-next-line @typescript-eslint/no-require-imports -- the fresh instance the module above reads
  const prefs = require("../preferences") as typeof import("../preferences");
  try {
    await prefs.setTaskFinishedReminders(false);
    // Off means the channel and the permission prompt are not prepared either:
    // an opted-out user must not meet the OS dialog for this feature.
    await module.prepareTaskNotifications();
    expect(notif.setNotificationChannelAsync).not.toHaveBeenCalled();
    expect(notif.getPermissionsAsync).not.toHaveBeenCalled();
    (notif.getPermissionsAsync as jest.Mock).mockResolvedValue({ granted: false, canAskAgain: true });
    await prefs.setTaskFinishedReminders(true);
    // The skip left the memo unset, so turning them on prepares for real.
    await module.prepareTaskNotifications();
    expect(notif.setNotificationChannelAsync).toHaveBeenCalledWith("task-completion", expect.objectContaining({ importance: 4 }));
    expect(notif.getPermissionsAsync).toHaveBeenCalledTimes(1);
  } finally {
    await prefs.setTaskFinishedReminders(true);
  }
});

type MockedNotifications = {
  setNotificationHandler: jest.Mock;
  setNotificationChannelAsync: jest.Mock;
  getPermissionsAsync: jest.Mock;
  addNotificationResponseReceivedListener: jest.Mock;
  getLastNotificationResponse: jest.Mock;
  clearLastNotificationResponse: jest.Mock;
};

/**
 * Re-import the module against fresh stand-ins, returning its registration.
 *
 * The module keeps per-run state (the notification handler it registers, and the
 * taps it has routed), so a test that wants a clean run has to re-import it —
 * and that re-imports everything it imports, which is why the fresh instances of
 * the alerts and the translations come back from here too.
 */
function freshModule() {
  jest.resetModules();
  const notif = jest.requireMock("expo-notifications") as MockedNotifications;
  const t = (jest.requireMock("../../i18n") as { default: { t: jest.Mock } }).default.t;
  // eslint-disable-next-line @typescript-eslint/no-require-imports -- fresh module instance
  const { AppAlert } = require("../../components/appAlerts") as typeof import("../../components/appAlerts");
  const alert = jest.spyOn(AppAlert, "alert").mockImplementation(() => {});
  // eslint-disable-next-line @typescript-eslint/no-require-imports -- fresh module instance
  const module = require("../taskNotifications") as typeof import("../taskNotifications");
  return { handler: notif.setNotificationHandler.mock.calls[0]?.[0], module, notif, t, alert };
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

/** A reminder response, in the shape expo-notifications hands back. */
function response(identifier: string, data?: Record<string, unknown>) {
  return {
    notification: { request: { identifier, content: { data } } },
    actionIdentifier: "expo.modules.notifications.actions.DEFAULT",
  };
}

/**
 * The module as one app run sees it.
 *
 * The tap routing is per app run (a response is remembered by identifier so one
 * tap is one open), so every test starts from a fresh module instance and
 * configures the launch response it wants *before* the app subscribes.
 */
function tapFixture() {
  const { module, notif, t, alert } = freshModule();
  const open = jest.fn(async () => {});
  return {
    module,
    notif,
    t,
    alert,
    open,
    subscribe: () => module.subscribeTaskNotificationTap(open),
    /** Deliver the response the way the OS does: to the registered listener. */
    tap: (value: unknown) =>
      (notif.addNotificationResponseReceivedListener.mock.calls[0]?.[0] as (r: unknown) => void)(value),
  };
}

describe("a tap on a task-completion reminder", () => {
  test("opens the conversation it names, on the machine that ran it", async () => {
    const fixture = tapFixture();
    fixture.subscribe();
    fixture.tap(response("n1", { desktopId: "desktop_2", sessionId: "b-1", machine: "Office" }));
    expect(fixture.open).toHaveBeenCalledWith({ desktopId: "desktop_2", sessionId: "b-1", machine: "Office" });
    // Read once: the tap is answered now, and the next launch (which remembers
    // no tap) must not open the conversation again.
    expect(fixture.notif.clearLastNotificationResponse).toHaveBeenCalledTimes(1);
  });

  test("the response that launched the app is read at subscription, after the listener is registered", () => {
    const fixture = tapFixture();
    // A launch by the tap has no listener yet when the response arrives: the
    // reminder's response *is* the last one, and only a read finds it.
    fixture.notif.getLastNotificationResponse.mockReturnValue(
      response("n7", { desktopId: "desktop_1", sessionId: "a-1", machine: "Studio" }),
    );
    fixture.subscribe();
    expect(fixture.open).toHaveBeenCalledWith({ desktopId: "desktop_1", sessionId: "a-1", machine: "Studio" });
    const listenerOrder = fixture.notif.addNotificationResponseReceivedListener.mock.invocationCallOrder[0]!;
    const readOrder = fixture.notif.getLastNotificationResponse.mock.invocationCallOrder[0]!;
    expect(listenerOrder).toBeLessThan(readOrder);
  });

  test("a response that arrives twice — as an event and as the launch read — opens once", () => {
    const fixture = tapFixture();
    const launched = response("n3", { desktopId: "desktop_1", sessionId: "a-1", machine: "Studio" });
    fixture.notif.getLastNotificationResponse.mockReturnValue(launched);
    fixture.subscribe();
    fixture.tap(launched);
    expect(fixture.open).toHaveBeenCalledTimes(1);
  });

  test("a later subscription does not replay a tap that was already answered", () => {
    const fixture = tapFixture();
    fixture.notif.getLastNotificationResponse.mockReturnValue(
      response("n5", { desktopId: "desktop_1", sessionId: "a-1", machine: "Studio" }),
    );
    const unsubscribe = fixture.subscribe();
    // The app re-subscribes whenever the machine it reads changes (the opener
    // closes over it); a re-subscription is not a second tap.
    unsubscribe();
    fixture.subscribe();
    expect(fixture.open).toHaveBeenCalledTimes(1);
  });

  test("a response that names no conversation, and a tap after unsubscribing, open nothing", () => {
    const fixture = tapFixture();
    const unsubscribe = fixture.subscribe();
    // Not one of ours: another app's notification, or a reminder scheduled by a
    // build whose payload carried no target.
    fixture.tap(response("n2"));
    fixture.tap(response("n9", { desktopId: "desktop_1" }));
    expect(fixture.open).not.toHaveBeenCalled();
    unsubscribe();
    fixture.tap(response("n4", { desktopId: "desktop_1", sessionId: "a-1" }));
    expect(fixture.open).not.toHaveBeenCalled();
  });

  test("a conversation that cannot be opened reports the machine the reminder named", async () => {
    const fixture = tapFixture();
    fixture.open.mockRejectedValue(new Error("desktop_unavailable"));
    fixture.subscribe();
    fixture.tap(response("n1", { desktopId: "desktop_2", sessionId: "b-1", machine: "Office" }));
    await Promise.resolve();
    // The tap is never left looking ignored: it says which machine it could not
    // reach, using the name the reminder itself showed.
    expect(fixture.t).toHaveBeenCalledWith("sessions.openOnDesktopFailed", { name: "Office" });
    expect(fixture.alert).toHaveBeenCalledWith("sessions.openOnDesktopFailed");
  });

  test("an opener that refuses synchronously is reported the same way", () => {
    const fixture = tapFixture();
    fixture.open.mockImplementation(() => {
      throw new Error("desktop_unavailable");
    });
    fixture.subscribe();
    fixture.tap(response("n1", { desktopId: "desktop_2", sessionId: "b-1", machine: "" }));
    // Without a name to show, the machine's id is what the report has.
    expect(fixture.t).toHaveBeenCalledWith("sessions.openOnDesktopFailed", { name: "desktop_2" });
  });

  test("a device whose native notifications expose no response API still subscribes", () => {
    const fixture = tapFixture();
    fixture.notif.addNotificationResponseReceivedListener.mockImplementation(() => {
      throw new Error("native unavailable");
    });
    fixture.notif.getLastNotificationResponse.mockImplementation(() => {
      throw new Error("native unavailable");
    });
    expect(() => fixture.subscribe()).not.toThrow();
    expect(fixture.open).not.toHaveBeenCalled();
  });
});
