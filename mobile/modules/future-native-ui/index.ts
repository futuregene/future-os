import { requireOptionalNativeModule } from "expo-modules-core";

interface NativeUiModule {
  showActionSheet(title: string | null, options: string[]): Promise<number | null>;
  isPhotoPickerAvailable?(): boolean;
}

const nativeModule = requireOptionalNativeModule<NativeUiModule>("FutureNativeUi");

/** Older native builds safely use the gallery intent, never the document fallback. */
export function isPhotoPickerAvailable(): boolean {
  return nativeModule?.isPhotoPickerAvailable?.() ?? false;
}

export async function showActionSheet(options: string[], title?: string): Promise<number | null> {
  if (!nativeModule) throw new Error("Native UI module is unavailable");
  return nativeModule.showActionSheet(title ?? null, options);
}
