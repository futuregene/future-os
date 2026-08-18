import { requireOptionalNativeModule } from "expo-modules-core";

interface FileHandlerNativeModule {
  findSupportedMimeType(fileName: string, mimeTypes: string[]): Promise<string | null>;
  openFile(fileUrl: string, mimeType: string): Promise<void>;
}

const nativeModule = requireOptionalNativeModule<FileHandlerNativeModule>("FutureFileHandler");

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
