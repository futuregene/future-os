import { requireOptionalNativeModule } from "expo-modules-core";

/** One activity that would answer an intent. */
export interface IntentHandler {
  package: string;
  activity: string;
}

/** Which apps answer each album/photo-picker intent on this Android device. */
export interface ImagePickRoutes {
  sdkInt: number;
  /** The classic gallery intent (ACTION_PICK on MediaStore's image collection). */
  album: IntentHandler[];
  /** ACTION_GET_CONTENT for an image, which galleries also register. */
  imageContent: IntentHandler[];
  /** Android 13's system photo picker. */
  photoPicker: IntentHandler[];
  /** The photo picker backport AOSP ships to Android 11/12 devices. */
  photoPickerFallback: IntentHandler[];
  /** The Play-services build of that backport. */
  photoPickerPlayServices: IntentHandler[];
  /** The document picker, which an album must never open. Diagnostics only. */
  document: IntentHandler[];
}

/** One image the album grid can offer, newest first within a listing. */
export interface AlbumImage {
  uri: string;
  name: string;
  mimeType: string;
  size: number;
  /** Epoch milliseconds, for a stable newest-first order. */
  modified: number;
}

interface FileHandlerNativeModule {
  hashFile?(fileUrl: string): Promise<string>;
  findSupportedMimeType(fileName: string, mimeTypes: string[]): Promise<string | null>;
  openFile(fileUrl: string, mimeType: string): Promise<void>;
  saveFile?(fileUrl: string): Promise<void>;
  shareFile?(fileUrl: string, mimeType: string, title: string): Promise<void>;
  resolveImagePickRoutes?(): Promise<ImagePickRoutes>;
  listAlbumImages?(limit: number): Promise<AlbumImage[]>;
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

/** `null` on iOS and on app binaries that predate the probe. */
export async function resolveImagePickRoutes(): Promise<ImagePickRoutes | null> {
  return (await nativeModule?.resolveImagePickRoutes?.()) ?? null;
}

/** Throws when the app binary has no album reader, so the caller can report the
 * album as unavailable rather than presenting an empty grid. */
export async function listAlbumImages(limit: number): Promise<AlbumImage[]> {
  if (!nativeModule?.listAlbumImages) {
    throw new Error("The album reader is unavailable; update the app");
  }
  return nativeModule.listAlbumImages(limit);
}

/** Whether this app binary can list the phone's images itself. */
export function supportsAlbumGrid(): boolean {
  return typeof nativeModule?.listAlbumImages === "function";
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
