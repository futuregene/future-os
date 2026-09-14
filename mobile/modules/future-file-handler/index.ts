import { requireOptionalNativeModule } from "expo-modules-core";

interface FileHandlerNativeModule {
  findSupportedMimeType(fileName: string, mimeTypes: string[]): Promise<string | null>;
  openFile(fileUrl: string, mimeType: string): Promise<void>;
  saveFile?(fileUrl: string): Promise<void>;
}

const nativeModule = requireOptionalNativeModule<FileHandlerNativeModule>("FutureFileHandler");

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
