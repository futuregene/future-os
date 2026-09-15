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
  /** The system could not provide a readable document. */
  failed?: boolean;
}

interface ShareIntentNativeModule {
  /** Take the pending share, or null when there is none. */
  getPendingShare(): Promise<SharedContent | null>;
  addListener?(event: "onPendingShare", listener: () => void): { remove(): void };
}

const nativeModule = requireOptionalNativeModule<ShareIntentNativeModule>("FutureShareIntent");

/** Notify a running app even if receiving a document causes no AppState change. */
export function addPendingShareListener(listener: () => void): { remove(): void } {
  // Old native builds can still rely on foreground reads.
  return nativeModule?.addListener?.("onPendingShare", listener) ?? { remove() {} };
}

/**
 * Read and clear the payload of the share intent that started or resumed the
 * app, including documents received through Open In. iOS checks its local
 * document inbox before the optional Share Extension's App Group inbox.
 * Resolves to null on older native builds or when nothing was shared.
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
