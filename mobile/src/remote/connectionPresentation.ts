import type { ConnectionPhase } from "./types";
import { remoteErrorPresentation } from "./errorPresentation";
import type { RemoteErrorAction, RemoteErrorCustomerState } from "./errorPresentation";

export type ConnectionLevel = "connected" | "connecting" | "disconnected";
export type ConnectionCustomerState =
  | "connected"
  | "connecting"
  | "waitingDesktop"
  | "devicePreparing"
  /**
   * The desktop has never answered this phone.
   *
   * Its own state — not "connecting": there an attempt or a recovery is
   * running, and here nothing is, because nothing has ever come back to run
   * one against. The red page a first connection failure gets is the right one
   * (a phone that has never reached this pairing cannot retry itself into a
   * desktop that is not serving it); this state is what makes that page say so
   * instead of the generic "the desktop may be asleep".
   */
  | "desktopNotServing"
  | "disconnected"
  | RemoteErrorCustomerState;
export type ConnectionAction =
  "none" | "wait" | "openDesktop" | "restartDesktop" | "reconnect" | RemoteErrorAction;

export interface ConnectionPresentation {
  level: ConnectionLevel;
  customerState: ConnectionCustomerState;
  action: ConnectionAction;
  supportCode: string | null;
  titleKey:
    | "connection.connected"
    | "connection.connecting"
    | "connection.waitingDesktop"
    | "connection.devicePreparing"
    | "connection.disconnected"
    | "connection.pairingExpired"
    | "connection.networkUnavailable"
    | "connection.serviceUnavailable"
    | "connection.deviceUnavailable"
    | "connection.connectionFailed";
  hintKey:
    | "connection.offlineHint"
    | "connection.manuallyDisconnectedHint"
    | "connection.agentUnavailable"
    | "connection.failedHint"
    | "connection.notServingHint"
    | null;
}

function failedTitleKey(state: RemoteErrorCustomerState): ConnectionPresentation["titleKey"] {
  switch (state) {
    case "networkUnavailable":
      return "connection.networkUnavailable";
    case "pairingExpired":
      return "connection.pairingExpired";
    case "serviceUnavailable":
      return "connection.serviceUnavailable";
    case "deviceUnavailable":
      return "connection.deviceUnavailable";
    case "contentUnavailable":
    case "operationFailed":
      return "connection.connectionFailed";
  }
}

/** Customer status describes availability; transport stages stay internal. */
export function connectionPresentation({
  phase,
  desktopOnline,
  agentAvailable = true,
  desktopDisconnected = false,
  desktopDisconnectReason,
  desktopUnanswered = false,
  error = null,
}: {
  phase: ConnectionPhase;
  desktopOnline: boolean;
  agentAvailable?: boolean;
  desktopDisconnected?: boolean;
  desktopDisconnectReason?: string;
  /**
   * Not one answer from this desktop in the first-answer budget.
   *
   * The transport's phases cannot say this by themselves: `connecting` and
   * `reconnecting` are the same two words for a first connection that is not
   * getting through and for a healthy connection on its way back. The
   * connection measures it (`useDesktopConnection`); here it is only the fact
   * that turns an endless "connecting" into a terminal the user can act on.
   * A phase that has an answer of its own — `ready`, an explicit disconnect, a
   * failure — outranks it.
   */
  desktopUnanswered?: boolean;
  error?: string | null;
}): ConnectionPresentation {
  if (phase === "revoked")
    return {
      level: "disconnected",
      customerState: "pairingExpired",
      action: "pairAgain",
      supportCode: "PA001",
      titleKey: "connection.pairingExpired",
      hintKey: null,
    };
  if (phase === "failed") {
    const failure = remoteErrorPresentation(error ?? "");
    return {
      level: "disconnected",
      customerState: failure.customerState,
      action: failure.action,
      supportCode: failure.supportCode,
      titleKey: failedTitleKey(failure.customerState),
      hintKey: "connection.failedHint",
    };
  }
  if (phase === "stopped" || phase === "unpaired" || desktopDisconnected)
    return {
      level: "disconnected",
      customerState: "disconnected",
      action: "reconnect",
      supportCode: desktopDisconnectReason === "system_sleep" ? "PW001" : null,
      titleKey: "connection.disconnected",
      hintKey: desktopDisconnectReason === "user_disconnect" ? "connection.manuallyDisconnectedHint" : "connection.offlineHint",
    };
  if (phase === "ready" && desktopOnline && agentAvailable)
    return {
      level: "connected",
      customerState: "connected",
      action: "none",
      supportCode: null,
      titleKey: "connection.connected",
      hintKey: null,
    };
  if (phase === "ready" && desktopOnline && !agentAvailable)
    return {
      level: "connecting",
      customerState: "devicePreparing",
      action: "restartDesktop",
      supportCode: "LC003",
      titleKey: "connection.waitingDesktop",
      hintKey: "connection.offlineHint",
    };
  if (phase === "ready")
    return {
      level: "connecting",
      customerState: "waitingDesktop",
      action: "openDesktop",
      supportCode: null,
      titleKey: "connection.waitingDesktop",
      hintKey: "connection.offlineHint",
    };
  // Nothing has ever come back from this desktop, and the budget for expecting
  // an answer is spent. This is a red terminal, not a yellow wait: the desktop
  // is not serving this device, and "nearly there" was the one thing the page
  // could say that was not true.
  if (desktopUnanswered)
    return {
      level: "disconnected",
      customerState: "desktopNotServing",
      action: "reconnect",
      supportCode: "RT007",
      titleKey: "connection.disconnected",
      hintKey: "connection.notServingHint",
    };
  return {
    level: "connecting",
    customerState: "connecting",
    action: "wait",
    supportCode: null,
    titleKey: "connection.connecting",
    hintKey: "connection.offlineHint",
  };
}
