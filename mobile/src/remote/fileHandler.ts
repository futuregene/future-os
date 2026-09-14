import { Platform } from "react-native";
import { findSupportedMimeType } from "future-file-handler";
import { externalMimeCandidates } from "./fileTypes";

/**
 * Resolve the MIME that an installed external app can consume. Android can
 * query VIEW intent handlers when the user chooses "open with another app".
 * Never use this to gate saving or sharing (SEND does not require a reader).
 * iOS does not expose an equivalent third-party document-handler query, so its
 * business allow-list is the boundary and the system document sheet decides.
 */
export async function supportedExternalMime(name: string): Promise<string | null> {
  const candidates = externalMimeCandidates(name);
  if (candidates.length === 0) return null;
  if (Platform.OS !== "android") return candidates[0]!;
  return findSupportedMimeType(name, candidates);
}
