import { requireOptionalNativeModule } from "expo-modules-core";

interface FileHandlerNativeModule {
  hashFile?(fileUrl: string): Promise<string>;
  findSupportedMimeType(fileName: string, mimeTypes: string[]): Promise<string | null>;
  openFile(fileUrl: string, mimeType: string): Promise<void>;
  saveFile?(fileUrl: string): Promise<void>;
  shareFile?(fileUrl: string, mimeType: string, title: string): Promise<void>;
}

const nativeModule = requireOptionalNativeModule<FileHandlerNativeModule>("FutureFileHandler");

/** Native streaming hash when available; older app binaries use the bounded
 * JS fallback. Native permission/read failures are not silently bypassed. */
export async function hashFile(fileUrl: string): Promise<string | null> {
  return nativeModule?.hashFile ? nativeModule.hashFile(fileUrl) : null;
}

/** Resolves when Android accepts the chooser, not when a recipient sends it. */
export async function shareFile(fileUrl: string, mimeType: string, title: string): Promise<void> {
  if (!nativeModule?.shareFile) throw new Error("Native file sharing is unavailable; update the app");
  await nativeModule.shareFile(fileUrl, mimeType, title);
}

/** Older iOS clients retain the system share-sheet fallback until rebuilt. */
export function supportsNativeFileActions(): boolean {
  return typeof nativeModule?.saveFile === "function";
}

export async function saveFile(fileUrl: string): Promise<void> {
  if (!nativeModule?.saveFile) throw new Error("Native document export is unavailable");
  await nativeModule.saveFile(fileUrl);
}

export async function findSupportedMimeType(
  fileName: string,
  mimeTypes: string[],
): Promise<string | null> {
  return nativeModule?.findSupportedMimeType(fileName, mimeTypes) ?? null;
}

export async function openFile(fileUrl: string, mimeType: string): Promise<void> {
  if (!nativeModule) throw new Error("File handler module is unavailable");
  await nativeModule.openFile(fileUrl, mimeType);
}
