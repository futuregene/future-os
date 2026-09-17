/**
 * Web stand-in for the `future-file-handler` native module: reports the file
 * actions a mobile build would offer so the action sheet renders for real.
 */
export function supportsNativeFileActions(): boolean {
  return true;
}

export async function openFile(): Promise<void> {}

export async function saveFile(): Promise<void> {}

export async function shareFile(): Promise<void> {}

export function findSupportedMimeType(...candidates: (string | null | undefined)[]): string | null {
  return candidates.find(candidate => typeof candidate === "string" && candidate.length > 0) ?? null;
}

export async function hashFile(): Promise<string> {
  return "0".repeat(64);
}
