import { Platform, ToastAndroid } from "react-native";
import { AppAlert as Alert } from "../../components/appAlerts";
import type { File } from "expo-file-system";
import type { DownloadInfo } from "../../remote/types";

export type DownloadPhase =
  | "preparing"
  | "downloading"
  | "waiting_network"
  | "verifying"
  | "saving"
  | "opening"
  | "sharing"
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

export type FileOperation = "open" | "save" | "share";

export interface FileAction {
  info: DownloadInfo;
  cachedFile: File | null;
}

export const MARKDOWN_RENDER_BYTES = 2 * 1024 * 1024;

// The fade band above the composer dock (styles.composerFade) is part of the
// dock's visual footprint: the list's bottom padding must clear it too, or a
// settled reply's footer ("time · tokens" + copy) rests under the
// semi-transparent gradient.
export const COMPOSER_FADE_CLEARANCE = 48;

// Transient failures (attachment pick, send) surface as a platform-native
// toast instead of pinned red text above the composer. iOS has no native
// toast, so it uses the shared app dialog.
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

/**
 * A spent amount in yuan, matching the desktop header/dialog formatting: up to
 * four decimals with trailing zeros dropped, thousands grouped with Latin
 * digits in both UI languages (a currency figure reads the same either way).
 */
export function formatCostCny(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "¥0";
  // A single request can cost ¥0.0004; the fourth decimal is the one that
  // moves, so `toFixed` rather than a 3-digit default.
  const rounded = value.toFixed(4);
  if (Number(rounded) === 0) return "¥<0.0001";
  const trimmed = rounded.replace(/0+$/, "").replace(/\.$/, "");
  const [whole, fraction] = trimmed.split(".");
  const grouped = new Intl.NumberFormat("en-US").format(Number(whole));
  return fraction ? `¥${grouped}.${fraction}` : `¥${grouped}`;
}

export function plainText(bytes: Uint8Array, truncated = false): string | null {
  // Mobile's fast-text-encoding fallback rejects fatal/stream options. Validate
  // UTF-8 ourselves before using the common no-options decoder, including a
  // preview boundary that splits an otherwise valid code point.
  let end = bytes.length;
  for (let i = 0; i < bytes.length;) {
    const first = bytes[i]!;
    if (first < 0x80) {
      if (first < 32 && first !== 9 && first !== 10 && first !== 13) return null;
      i++;
      continue;
    }
    const count = first >= 0xc2 && first <= 0xdf ? 2
      : first >= 0xe0 && first <= 0xef ? 3
        : first >= 0xf0 && first <= 0xf4 ? 4 : 0;
    if (!count) return null;
    for (let j = 1; j < count && i + j < bytes.length; j++) {
      const next = bytes[i + j]!;
      if (next < 0x80 || next > 0xbf) return null;
      if (j === 1 && ((first === 0xe0 && next < 0xa0) || (first === 0xed && next >= 0xa0)
        || (first === 0xf0 && next < 0x90) || (first === 0xf4 && next >= 0x90))) return null;
    }
    if (i + count > bytes.length) {
      if (!truncated) return null;
      end = i;
      break;
    }
    i += count;
  }
  return new TextDecoder().decode(bytes.subarray(0, end));
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
