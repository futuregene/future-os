import { Linking, Platform } from "react-native";
import { File, Paths } from "expo-file-system";
import { getContentUriAsync } from "expo-file-system/legacy";
import * as IntentLauncher from "expo-intent-launcher";
import { VERSION } from "../version.generated";

// iOS reads the live App Store version from Apple directly (the App Store lags
// behind latest.json because of review), while Android reads the shared release
// manifest — mirroring how the desktop resolves updates.
export const ITUNES_LOOKUP_URL = "https://itunes.apple.com/lookup?bundleId=cn.futureos.mobile";
export const UPDATE_MANIFEST_URL = "https://dl.future-os.cn/releases/latest.json";

export interface UpdateStatus {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  appStoreUrl: string | null;
  downloadUrl: string | null;
}

type FetchLike = typeof fetch;

function semverCore(version: string): string {
  return version.split(/[-+]/)[0] ?? version;
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
  };
}

export async function checkAndroidUpdate(
  currentVersion: string,
  fetchFn: FetchLike = fetch,
): Promise<UpdateStatus> {
  const response = await fetchFn(UPDATE_MANIFEST_URL);
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
      compareVersions(latestVersion, currentVersion) > 0,
    appStoreUrl: null,
    downloadUrl,
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
