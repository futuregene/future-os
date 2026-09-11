export type RemoteErrorCustomerState =
  | "networkUnavailable"
  | "pairingExpired"
  | "contentUnavailable"
  | "serviceUnavailable"
  | "deviceUnavailable"
  | "operationFailed";

export type RemoteErrorAction = "checkNetwork" | "pairAgain" | "retry" | "contactSupport";

export interface RemoteErrorPresentation {
  customerState: RemoteErrorCustomerState;
  action: RemoteErrorAction;
  messageKey:
    | "connection.errorPairing"
    | "connection.errorNotFound"
    | "connection.errorRateLimit"
    | "connection.errorServiceLater"
    | "connection.errorServiceSupport"
    | "connection.errorDeviceLater"
    | "connection.errorNetwork"
    | "connection.errorGeneric";
  supportCode: string;
}

/** Convert raw backend/transport detail into a stable customer category and support code. */
export function remoteErrorPresentation(message: string): RemoteErrorPresentation {
  const trimmed = message.trim();
  const code = /^(?:HTTP\s*)?(\d{3})$/.exec(trimmed)?.[1];
  if (code) {
    if (code === "401" || code === "403")
      return {
        customerState: "pairingExpired",
        action: "pairAgain",
        messageKey: "connection.errorPairing",
        supportCode: "PA002",
      };
    if (code === "404")
      return {
        customerState: "contentUnavailable",
        action: "retry",
        messageKey: "connection.errorNotFound",
        supportCode: "DT001",
      };
    if (code === "429")
      return {
        customerState: "serviceUnavailable",
        action: "retry",
        messageKey: "connection.errorRateLimit",
        supportCode: "SV002",
      };
    if (code.startsWith("5"))
      return {
        customerState: "serviceUnavailable",
        action: "retry",
        messageKey: "connection.errorServiceLater",
        supportCode: "SV001",
      };
  }
  if (/agent.*(offline|unavailable)|history is unavailable/i.test(trimmed))
    return {
      customerState: "deviceUnavailable",
      action: "retry",
      messageKey: "connection.errorDeviceLater",
      supportCode: "LC003",
    };
  if (/time-?out|timed out/i.test(trimmed))
    return {
      customerState: "networkUnavailable",
      action: "checkNetwork",
      messageKey: "connection.errorNetwork",
      supportCode: "NW002",
    };
  if (/generation_unhealthy:protocol/i.test(trimmed))
    return {
      customerState: "serviceUnavailable",
      action: "contactSupport",
      messageKey: "connection.errorServiceSupport",
      supportCode: "PT001",
    };
  if (/generation_unhealthy:subscription/i.test(trimmed))
    return {
      customerState: "serviceUnavailable",
      action: "retry",
      messageKey: "connection.errorServiceLater",
      supportCode: "RT001",
    };
  if (/remote_service_misconfigured|permissions_violation|authorization_violation/i.test(trimmed))
    return {
      customerState: "serviceUnavailable",
      action: "contactSupport",
      messageKey: "connection.errorServiceSupport",
      supportCode: "AU001",
    };
  if (
    /network|unreachable|load failed|fetch failed|econn|not_connected|nats_|subscription_ended|connection (refused|reset)/i.test(
      trimmed,
    )
  )
    return {
      customerState: "networkUnavailable",
      action: "checkNetwork",
      messageKey: "connection.errorNetwork",
      supportCode: "NW001",
    };
  if (
    /invalid_remote_credential|credentials_revoked|invalid_jwt|pairing_signature|confirmation_mismatch/i.test(
      trimmed,
    )
  )
    return {
      customerState: "pairingExpired",
      action: "pairAgain",
      messageKey: "connection.errorPairing",
      supportCode: "PA001",
    };
  return {
    customerState: "operationFailed",
    action: "retry",
    messageKey: "connection.errorGeneric",
    supportCode: "LC999",
  };
}
