import type { TFunction } from "i18next";
import type { RemoteDevice } from "./remoteClient";

/**
 * Shared presentation rules for the host direction.
 *
 * Kept out of the components so both the list and a client's own page describe
 * the same device the same way — and so the view modules only export components
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
 */
export function isLimitError(err: unknown): boolean {
  const text = err instanceof Error ? err.message : String(err);
  return text.includes("pairing_limit_reached");
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

/** The status of one client, as a badge tone + label pair. */
export function deviceState(
  device: RemoteDevice,
  t: TFunction<"remote">,
): { label: string; tone: "accent" | "warning" | "neutral" | "info" } {
  if (device.control === "active") {
    return { label: t("deviceActive"), tone: "accent" };
  }
  if (device.pairingCode != null) {
    return { label: t("deviceWaiting"), tone: "info" };
  }
  if (device.connected) {
    return { label: t("deviceStandby"), tone: "warning" };
  }
  return { label: t("deviceOffline"), tone: "neutral" };
}
