import * as Network from "expo-network";

// Explicit, user-initiated transfers below 1 MiB should not interrupt reading.
// Cached files bypass this policy entirely; this is not permission to prefetch.
export const DOWNLOAD_CONFIRM_BYTES = 1024 * 1024;

export async function downloadWarning(size: number): Promise<
  "attachment.cellularWarning" | "attachment.unknownNetworkWarning" | null
> {
  if (Number.isFinite(size) && size >= 0 && size < DOWNLOAD_CONFIRM_BYTES) return null;
  try {
    const network = await Network.getNetworkStateAsync();
    if (network.type === Network.NetworkStateType.CELLULAR) return "attachment.cellularWarning";
    if (network.type === Network.NetworkStateType.WIFI || network.type === Network.NetworkStateType.ETHERNET) return null;
  } catch {
    // Some Android compatibility runtimes cannot report the network type.
    // Keep large transfers usable, without silently assuming Wi-Fi.
  }
  return "attachment.unknownNetworkWarning";
}
