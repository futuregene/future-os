import { requireOptionalNativeModule } from "expo-modules-core";

/** A file the user shared into the app, copied into app-owned storage. */
export interface SharedFile {
  /** `file://` URL inside the app's cache directory. */
  uri: string;
  name: string;
  mimeType: string;
}

/** The payload of a share intent addressed to this app. */
export interface SharedContent {
  text: string;
  files: SharedFile[];
  /** A shared file was dropped for exceeding the size ceiling. */
  tooLarge: boolean;
}

interface ShareIntentNativeModule {
  /** Take the pending share, or null when there is none. */
  getPendingShare(): Promise<SharedContent | null>;
}

const nativeModule = requireOptionalNativeModule<ShareIntentNativeModule>("FutureShareIntent");

/**
 * Read and clear the payload of the share intent that started or resumed the
 * app. Resolves to null when the platform has no share intake (iOS: a share
 * extension target is not part of this build) or when nothing was shared.
 */
export async function getPendingShare(): Promise<SharedContent | null> {
  if (!nativeModule) return null;
  try {
    return await nativeModule.getPendingShare();
  } catch {
    // A malformed share must never keep the app from starting.
    return null;
  }
}
