import { Alert, Platform, ToastAndroid } from "react-native";
import type { File } from "expo-file-system";
import type { DownloadInfo } from "../../remote/types";

export type DownloadPhase =
  | "preparing"
  | "downloading"
  | "waiting_network"
  | "verifying"
  | "saving"
  | "opening"
  | "cancelling";

export interface ActiveDownload {
  id: string;
  fileName: string;
  phase: DownloadPhase;
  completedBytes: number;
  totalBytes: number;
}

export interface DownloadHandle {
  id: string;
  fileName: string;
  visible: boolean;
  controller: AbortController;
  handoffPending: boolean;
}

export interface FileAction {
  info: DownloadInfo;
  cachedFile: File | null;
  openMimeType: string;
}

export const MARKDOWN_RENDER_BYTES = 2 * 1024 * 1024;

// The fade band above the composer dock (styles.composerFade) is part of the
// dock's visual footprint: the list's bottom padding must clear it too, or a
// settled reply's footer ("time · tokens" + copy) rests under the
// semi-transparent gradient.
export const COMPOSER_FADE_CLEARANCE = 48;

// Transient failures (attachment pick, send) surface as a platform-native
// toast instead of pinned red text above the composer. iOS has no native
// toast, so it falls back to a plain Alert like the rest of the app's errors.
export function showToast(message: string): void {
  if (Platform.OS === "android") {
    ToastAndroid.show(message, ToastAndroid.SHORT);
  } else {
    Alert.alert(message);
  }
}

export function deferPresentation(action: () => void): void {
  // UIKit invokes action-sheet callbacks before the dismissal animation has
  // fully released its presentation controller. A short delay avoids racing
  // the next native controller. InteractionManager is deliberately avoided:
  // it can remain pending while a Modal is itself transitioning.
  setTimeout(action, Platform.OS === "ios" ? 350 : 0);
}

export function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${Math.max(1, Math.ceil(bytes / 1024))} KB`;
}

export function plainText(bytes: Uint8Array): string | null {
  // Binary formats such as PDF contain NUL or C0 control bytes. The desktop
  // repeats a stricter UTF-8 check before it transfers a durable attachment.
  if (bytes.some(byte => byte === 0 || (byte < 32 && byte !== 9 && byte !== 10 && byte !== 13))) {
    return null;
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

export function confirmDownload(title: string, message: string, cancel: string, download: string) {
  return new Promise<boolean>(resolve => {
    let settled = false;
    const settleAfterDismissal = (accepted: boolean) => {
      if (settled) return;
      settled = true;
      // Alert button callbacks can run before UIKit has released its
      // presentation controller. Continue only after the dismissal window so
      // the progress Modal never competes with the cellular-confirmation alert.
      deferPresentation(() => resolve(accepted));
    };
    Alert.alert(
      title,
      message,
      [
        { text: cancel, style: "cancel", onPress: () => settleAfterDismissal(false) },
        { text: download, onPress: () => settleAfterDismissal(true) },
      ],
      { cancelable: true, onDismiss: () => settleAfterDismissal(false) },
    );
  });
}
