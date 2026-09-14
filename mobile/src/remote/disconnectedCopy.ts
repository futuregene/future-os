import type { ConnectionPresentation } from "./connectionPresentation";

/** Copy for the disconnected page: one cause, followed by one next step. */
export function disconnectedCopy(
  presentation: ConnectionPresentation,
  reason?: string,
  error?: string | null,
) {
  if (presentation.customerState === "disconnected") {
    switch (reason) {
      case "user_disconnect": return "manual";
      case "system_sleep": return "sleep";
      case "app_exit": return "exit";
      case "system_power_off": return "powerOff";
      default: return "offline";
    }
  }
  if (error === "recovery_timeout") return "recoveryTimeout";
  if (presentation.supportCode === "NW002") return "timeout";
  if (presentation.supportCode === "SV002") return "busy";
  if (presentation.action === "pairAgain") return "pairing";
  if (presentation.action === "contactSupport") return "support";
  switch (presentation.customerState) {
    case "networkUnavailable": return "network";
    case "deviceUnavailable": return "device";
    case "serviceUnavailable": return "service";
    case "contentUnavailable": return "content";
    default: return "failed";
  }
}
