import { requireOptionalNativeModule } from "expo-modules-core";

interface NativeUiModule {
  showActionSheet(title: string | null, options: string[]): Promise<number | null>;
}

const nativeModule = requireOptionalNativeModule<NativeUiModule>("FutureNativeUi");

export async function showActionSheet(options: string[], title?: string): Promise<number | null> {
  if (!nativeModule) throw new Error("Native UI module is unavailable");
  return nativeModule.showActionSheet(title ?? null, options);
}
