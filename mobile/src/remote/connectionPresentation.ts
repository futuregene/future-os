import type { ConnectionPhase } from "./types";
import { remoteErrorPresentation } from "./errorPresentation";
import type { RemoteErrorAction, RemoteErrorCustomerState } from "./errorPresentation";

export type ConnectionLevel = "connected" | "connecting" | "disconnected";
export type ConnectionCustomerState =
  | "connected"
  | "connecting"
  | "waitingDesktop"
  | "devicePreparing"
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
    "connection.offlineHint" | "connection.agentUnavailable" | "connection.failedHint" | null;
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
  error = null,
}: {
  phase: ConnectionPhase;
  desktopOnline: boolean;
  agentAvailable?: boolean;
  desktopDisconnected?: boolean;
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
  if (phase === "unpaired" || desktopDisconnected)
    return {
      level: "disconnected",
      customerState: "disconnected",
      action: "reconnect",
      supportCode: null,
      titleKey: "connection.disconnected",
      hintKey: "connection.offlineHint",
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
      titleKey: "connection.devicePreparing",
      hintKey: "connection.agentUnavailable",
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
  return {
    level: "connecting",
    customerState: "connecting",
    action: "wait",
    supportCode: null,
    titleKey: "connection.connecting",
    hintKey: "connection.offlineHint",
  };
}
