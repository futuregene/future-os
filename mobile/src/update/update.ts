import { Linking, Platform } from "react-native";
import { File, Paths } from "expo-file-system";
import { getContentUriAsync } from "expo-file-system/legacy";
import * as IntentLauncher from "expo-intent-launcher";
import { VERSION } from "../version.generated";

// iOS reads the live App Store version from Apple directly (the App Store lags
// behind latest.json because of review). Android selects one of the two static
// manifests from its embedded FutureOS version.
export const ITUNES_LOOKUP_URL = "https://itunes.apple.com/lookup?bundleId=cn.futureos.mobile";
export const RELEASE_MANIFEST_URL = "https://dl.future-os.cn/releases/latest.json";
export const NIGHTLY_MANIFEST_URL = "https://dl.future-os.cn/nightly/latest.json";

export type BuildChannel = "release" | "test" | "nightly" | "dev" | "local";

export interface UpdateStatus {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  appStoreUrl: string | null;
  downloadUrl: string | null;
  /** Whether FutureOS may download the package and invoke its installer. */
  canInstallInApp: boolean;
}

type FetchLike = typeof fetch;

function semverCore(version: string): string {
  return version.split(/[-+]/)[0] ?? version;
}

/** Unknown 0.* versions stay on the safe, manual-only dev policy. */
export function buildChannel(version: string): BuildChannel {
  if (!version.startsWith("0")) return "release";
  const metadata = version.split("+", 2)[1];
  if (metadata === "test" || metadata === "nightly" || metadata === "dev") return metadata;
  if (metadata === "local" || metadata === "local.dirty") return "local";
  return "dev";
}

export function manifestUrl(version: string): string {
  return buildChannel(version) === "release" ? RELEASE_MANIFEST_URL : NIGHTLY_MANIFEST_URL;
}

function channelRunNumber(version: string): number | null {
  const match = /^\d+\.\d+\.\d+-(\d+)\+(?:test|nightly)$/.exec(version);
  if (!match) return null;
  const run = Number.parseInt(match[1]!, 10);
  return Number.isSafeInteger(run) ? run : null;
}

export function shouldOfferAndroidUpdate(currentVersion: string, latestVersion: string): boolean {
  const channel = buildChannel(currentVersion);
  const latestChannel = buildChannel(latestVersion);
  if (channel === "release") {
    return latestChannel === "release" && compareVersions(latestVersion, currentVersion) > 0;
  }
  if (channel === "test" || channel === "nightly") {
    if (latestChannel !== "nightly") return false;
    const coreOrder = compareVersions(latestVersion, currentVersion);
    if (coreOrder !== 0) return coreOrder > 0;
    const currentRun = channelRunNumber(currentVersion);
    const latestRun = channelRunNumber(latestVersion);
    return currentRun !== null && latestRun !== null && latestRun > currentRun;
  }
  return latestChannel === "nightly" && latestVersion !== currentVersion;
}

/** Numeric comparison of the semver core (prerelease/build suffixes ignored). */
export function compareVersions(a: string, b: string): number {
  const left = semverCore(a)
    .split(".")
    .map(part => Number.parseInt(part, 10) || 0);
  const right = semverCore(b)
    .split(".")
    .map(part => Number.parseInt(part, 10) || 0);
  const length = Math.max(left.length, right.length);
  for (let i = 0; i < length; i += 1) {
    const l = left[i] ?? 0;
    const r = right[i] ?? 0;
    if (l < r) return -1;
    if (l > r) return 1;
  }
  return 0;
}

export async function checkIosUpdate(
  currentVersion: string,
  fetchFn: FetchLike = fetch,
): Promise<UpdateStatus> {
  const response = await fetchFn(ITUNES_LOOKUP_URL);
  if (!response.ok) throw new Error(`lookup failed: ${response.status}`);
  const data = (await response.json()) as {
    results?: { version?: string; trackViewUrl?: string }[];
  };
  const result = data.results?.[0];
  const latestVersion = result?.version ?? null;
  const appStoreUrl = result?.trackViewUrl ?? null;
  return {
    currentVersion,
    latestVersion: latestVersion ?? currentVersion,
    hasUpdate:
      latestVersion !== null &&
      appStoreUrl !== null &&
      compareVersions(latestVersion, currentVersion) > 0,
    appStoreUrl,
    downloadUrl: null,
    canInstallInApp: true,
  };
}

export async function checkAndroidUpdate(
  currentVersion: string,
  fetchFn: FetchLike = fetch,
): Promise<UpdateStatus> {
  const response = await fetchFn(manifestUrl(currentVersion));
  if (!response.ok) throw new Error(`manifest failed: ${response.status}`);
  const manifest = (await response.json()) as {
    version?: string;
    assets?: { android?: { url?: string } };
  };
  const latestVersion = manifest.version ?? null;
  const downloadUrl = manifest.assets?.android?.url ?? null;
  return {
    currentVersion,
    latestVersion: latestVersion ?? currentVersion,
    hasUpdate:
      latestVersion !== null &&
      downloadUrl !== null &&
      shouldOfferAndroidUpdate(currentVersion, latestVersion),
    appStoreUrl: null,
    downloadUrl,
    canInstallInApp: ["release", "test", "nightly"].includes(buildChannel(currentVersion)),
  };
}

export function checkForUpdate(
  currentVersion: string = VERSION,
  fetchFn: FetchLike = fetch,
): Promise<UpdateStatus> {
  if (Platform.OS === "ios") return checkIosUpdate(currentVersion, fetchFn);
  return checkAndroidUpdate(currentVersion, fetchFn);
}

export async function installUpdate(status: UpdateStatus): Promise<void> {
  if (Platform.OS === "ios") {
    if (!status.appStoreUrl) throw new Error("App Store URL is unavailable");
    await Linking.openURL(status.appStoreUrl);
    return;
  }
  if (!status.downloadUrl) throw new Error("Download URL is unavailable");
  if (!status.canInstallInApp) {
    await Linking.openURL(status.downloadUrl);
    return;
  }
  const file = await File.downloadFileAsync(
    status.downloadUrl,
    new File(Paths.cache, "futureos-update.apk"),
    { idempotent: true },
  );
  const contentUri = await getContentUriAsync(file.uri);
  await IntentLauncher.startActivityAsync("android.intent.action.VIEW", {
    data: contentUri,
    flags: 1,
    type: "application/vnd.android.package-archive",
  });
}
