import type { TFunction } from "i18next";
import type { RemoteDevice } from "./remoteClient";

/**
 * Shared presentation rules for the host direction.
 *
 * Kept out of the components so the list and the confirm dialogs it opens
 * describe the same device the same way — and so the view modules only export components
 * (a module's fast-refresh boundary is its component exports).
 */

export function formatCountdown(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

/**
 * Short display form of a pair id: drop the `pair_` prefix (redundant next to the
 * "Paired as" label) and upper-case the remainder so it reads as a code, not a
 * lowercase hash. Unknown shapes pass through unchanged.
 */
export function formatPairId(pairId: string | null | undefined): string {
  if (!pairId)
    return "";
  return (pairId.startsWith("pair_") ? pairId.slice(5) : pairId).toUpperCase();
}

/**
 * The platform's cap is a count over rows it owns, so the only honest place to
 * learn that a device cannot be added is the refusal itself.
 *
 * Three shapes reach here, because an `AppError` serializes as a display string
 * and the typed code can be lost on the way:
 *
 * - `pairing_limit_reached` — what the command now reports, translated from the
 *   typed refusal;
 * - the platform's sentence ("This desktop already holds 5 devices…"), which is
 *   all the display string carries when the code is not translated;
 * - the code inside a Debug-shaped error, which older builds produced.
 */
export function isLimitError(err: unknown): boolean {
  const text = err instanceof Error ? err.message : String(err);
  return /pairing_limit_reached|already holds \d+ devices|maximum number of devices/i.test(text);
}

/**
 * The sentence to show when asking for a new invitation fails.
 *
 * "Add a device" is two round trips — minting the code on the platform, then
 * connecting the bridge that will serve it — and the serialized error the
 * frontend receives is the backend's plain message. Collapsing every one of them
 * into the same "try again later" told the user nothing they could act on: an
 * offline laptop, a stale sign-in and a self-hosted relay that is not up yet all
 * looked identical. The classes below are the ones with a different next step;
 * anything unrecognised keeps the neutral sentence, because there is nothing
 * honest to add.
 */
export function addDeviceErrorText(err: unknown, t: TFunction<"remote">): string {
  if (isLimitError(err))
    return t("error.limitReached");
  const message = err instanceof Error ? err.message : String(err ?? "");
  if (/not signed in/i.test(message))
    return t("failure.account");
  if (/authorization violation|remote_authorization|unauthorized/i.test(message))
    return t("failure.serviceSupport", { code: "AU001" });
  if (/failed to connect to nats|nats_connect|connection refused|connection reset|network is unreachable|lookup host|name or service not known|broken pipe|timed out|timed_out|deadline/i.test(message))
    return t("failure.network", { code: "NW001" });
  // A gateway error carries no platform body, so the status is all that reaches
  // here — `Failed to create pairing code (HTTP 502)`. That is the service
  // failing, not this machine: the same request succeeds seconds later, and
  // telling the user their device faulted sends them looking for the wrong
  // problem. Matches the `SV001` the status page already uses for it.
  if (/\(HTTP 5\d\d\)/i.test(message))
    return t("failure.serviceLater", { code: "SV001" });
  return t("error.generic");
}

/** What to call a device: what it said it was, else what we can see. */
export function deviceLabel(
  device: Pick<RemoteDevice, "client" | "connected">,
  t: TFunction<"remote">,
): string {
  if (device.client) {
    return t(`clientKind.${device.client.kind}`, {
      defaultValue: t("clientKind.other"),
      name: device.client.name,
    });
  }
  return device.connected ? t("clientUnknown") : t("devicePaired");
}

/**
 * Whether the machine can actually serve this device right now.
 *
 * Two halves, and both are needed: the bridge (`connected`) and the client that
 * is supposed to be behind it (`present`). A row that read only the bridge said
 * "in control" about a device whose user had just disconnected — the bridge was
 * healthy, and nobody was on the other side of it.
 */
export function deviceReady(device: RemoteDevice): boolean {
  return device.connected && device.present !== false;
}

/** The status of one client, as a badge tone + label pair. */
export function deviceState(
  device: RemoteDevice,
  t: TFunction<"remote">,
): { label: string; tone: "accent" | "warning" | "neutral" | "info" } {
  if (device.pairingCode != null) {
    return { label: t("deviceWaiting"), tone: "info" };
  }
  // A device that is not there is not controlling anything, whatever the
  // machine's control pointer still says. Saying otherwise is what made the row
  // keep reading "in control" after the client disconnected.
  if (!deviceReady(device)) {
    return { label: t("deviceOffline"), tone: "neutral" };
  }
  if (device.control === "active") {
    return { label: t("deviceActive"), tone: "accent" };
  }
  return { label: t("deviceStandby"), tone: "warning" };
}
