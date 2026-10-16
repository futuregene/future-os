import { AppState, Platform } from "react-native";
import { AppAlert as Alert } from "../components/appAlerts";
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

/**
 * Where a tapped reminder says the finished conversation is.
 *
 * A reminder is about one conversation on one machine, so the tap has to land on
 * it: the phone can hold conversations with the same title on several desktops,
 * and "the app opened" is not an answer to "which one finished".
 */
export interface TaskNotificationTarget {
  /** The paired machine that ran the task. */
  desktopId: string;
  sessionId: string;
  /**
   * What that machine was called when the reminder was scheduled.
   *
   * Carried with the reminder rather than looked up on the tap: the machine a
   * tap cannot reach is often the one that is no longer paired, which is exactly
   * when the phone can no longer name it.
   */
  machine: string;
}

/** The machine a completion reminder belongs to. */
export interface TaskNotificationContext {
  /**
   * The user's name for the machine the conversation is on.
   *
   * A phone can hold conversations with the same title on several desktops, and
   * a notification that does not say which machine ran is a notification the
   * reader has to go and disambiguate.
   */
  machine?: string;
  /**
   * That machine's id. The phone may have moved to another machine by the time
   * the reminder is tapped, so the id travels with the reminder and the tap opens
   * the conversation where it lives rather than wherever the screens happen to be.
   */
  desktopId?: string;
}

/** No message content in lock-screen notifications; only the conversation title. */
export async function notifyTaskFinished(
  session: RemoteSession,
  isCurrent: () => boolean,
  context: TaskNotificationContext = {},
): Promise<void> {
  const { machine, desktopId } = context;
  const title = i18n.t(session.status === "failed" ? "notifications.failed" : "notifications.completed");
  const conversation = session.title || i18n.t("sessions.unnamed");
  const body = machine ? `${machine} · ${conversation}` : conversation;
  try {
    await prepareTaskNotifications();
    const permission = await Notifications.getPermissionsAsync();
    if (!isCurrent()) return;
    if (permission.granted || permission.ios?.status === Notifications.IosAuthorizationStatus.PROVISIONAL) {
      await Notifications.scheduleNotificationAsync({
        content: {
          title,
          body,
          sound: "default",
          // What a tap on this reminder does (see `subscribeTaskNotificationTap`):
          // the payload is the only thing the OS hands back.
          data: { desktopId: desktopId ?? "", sessionId: session.sessionId, machine: machine ?? "" },
        },
        trigger: Platform.OS === "android" ? { channelId: CHANNEL_ID } : null,
      });
      return;
    }
  } catch {
    // Permission refusal/native delivery failure must not break remote sync.
  }
  if (isCurrent() && AppState.currentState === "active") Alert.alert(title, body);
}

/** Opens a conversation on the machine that owns it. */
export type TaskNotificationOpener = (target: TaskNotificationTarget) => Promise<void>;

/** The openers a tap is routed to. The app registers one; a tap reaches all. */
const openers = new Set<TaskNotificationOpener>();
/** Responses already routed, by expo's own per-notification identifier. */
const routed = new Set<string>();
let listening = false;

/**
 * Route taps on task-completion reminders to the conversation they finished in.
 *
 * The OS owns the notification, so a tap can arrive on a cold start, from the
 * background, or over any screen: none of it is app state, and both of the
 * surprises below have to be handled here rather than in a caller.
 *
 * One tap is one open. expo-notifications delivers a response twice over — as an
 * event, and as the "last response" a launch by the tap has to read because it
 * had no listener yet — and a platform may replay it again. Responses are
 * therefore remembered by identifier and routed once.
 *
 * A tap that opens nothing says why. An opener that cannot reach its machine
 * (unpaired since, or not answering) reports the machine it could not reach
 * instead of leaving the tap looking ignored — which is the whole failure this
 * path exists to remove.
 */
export function subscribeTaskNotificationTap(open: TaskNotificationOpener): () => void {
  openers.add(open);
  listen();
  return () => {
    openers.delete(open);
  };
}

/** Register the native listener once, and read the tap that may have launched us. */
function listen(): void {
  if (listening) return;
  listening = true;
  try {
    Notifications.addNotificationResponseReceivedListener(routeResponse);
  } catch {
    // Native notifications may be unavailable (e.g. an older development
    // client). The reminder still arrives; a tap then opens the app, as before.
  }
  // A launch made *by* the tap has no listener when the response arrives, so the
  // response that launched the app is the last one. Read after registering the
  // listener: a tap landing between the two is then caught by one of the paths.
  routeResponse(lastResponse());
}

function lastResponse(): Notifications.NotificationResponse | null {
  try {
    return Notifications.getLastNotificationResponse();
  } catch {
    // Same older-client case as above: no response to read.
    return null;
  }
}

function routeResponse(response: Notifications.NotificationResponse | null): void {
  if (!response) return;
  const identifier = response.notification.request.identifier;
  if (routed.has(identifier)) return;
  routed.add(identifier);
  // Handled once, read once: the next launch of the app must not re-open a
  // conversation whose tap happened days ago.
  try {
    Notifications.clearLastNotificationResponse();
  } catch {
    // Nothing to clear.
  }
  const target = targetOf(response);
  if (!target) return;
  for (const open of openers) openConversation(open, target);
}

function targetOf(response: Notifications.NotificationResponse): TaskNotificationTarget | null {
  const data = response.notification.request.content.data;
  const sessionId = data?.sessionId;
  // Not one of ours: a reminder scheduled before the payload carried a target
  // (an older build), or some other app's notification.
  if (typeof sessionId !== "string" || !sessionId) return null;
  return {
    desktopId: typeof data.desktopId === "string" ? data.desktopId : "",
    sessionId,
    machine: typeof data.machine === "string" ? data.machine : "",
  };
}

function openConversation(open: TaskNotificationOpener, target: TaskNotificationTarget): void {
  const reportFailure = () => {
    Alert.alert(i18n.t("sessions.openOnDesktopFailed", {
      name: target.machine || target.desktopId,
    }));
  };
  try {
    void open(target).catch(reportFailure);
  } catch {
    // An opener that refuses synchronously is a failure like any other.
    reportFailure();
  }
}
