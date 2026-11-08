import AsyncStorage from "@react-native-async-storage/async-storage";

/**
 * The version the user last chose to skip.
 *
 * The reminder's in-memory set only survives the mounted component, so a phone
 * that is opened three times a day asked the same question three times: tapping
 * "later" meant "not now" for exactly one launch. What the user dismissed is a
 * build identity, so it is remembered here as the version string — a newer one
 * asks again, the same one stays quiet.
 *
 * A read failure must count as "never dismissed": unreadable storage may nag,
 * never silence an update. A write failure is swallowed for the same reason —
 * the dialog is already closed, and the next dismissal retries the write.
 */
export const DISMISSED_VERSION_KEY = "futureos.mobile.update-dismissed.v1";

export async function loadDismissedVersion(): Promise<string | null> {
  try {
    const raw = await AsyncStorage.getItem(DISMISSED_VERSION_KEY);
    return raw && raw.length > 0 ? raw : null;
  } catch {
    return null;
  }
}

export async function saveDismissedVersion(version: string): Promise<void> {
  try {
    await AsyncStorage.setItem(DISMISSED_VERSION_KEY, version);
  } catch {
    // Nothing to undo: the dialog is closed and the in-memory set still holds.
  }
}
