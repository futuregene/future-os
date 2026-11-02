import type { FusionSettings, FusionSettingsPatch } from "@future-os/thread-projection";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { useTauriEvent } from "../../lib/useTauriEvent";
import { invokeCommand } from "../tauri/invoke";

export const getFusionSettings = () => invokeCommand<FusionSettings>("get_fusion_settings");
export const updateFusionSettings = (patch: FusionSettingsPatch) => invokeCommand<FusionSettings>("update_fusion_settings", { input: patch });
export function useFusionAvailability() {
  const resource = useAsyncResource(getFusionSettings, [], null);
  useTauriEvent("app_settings_changed", resource.reload);
  useTauriEvent("provider-config-changed", resource.reload);
  return resource.data?.enabled ?? false;
}
