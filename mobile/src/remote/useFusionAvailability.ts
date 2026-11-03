import { useDesktopResource } from "../features/settings/useDesktopResource";
import { useRemoteControls } from "./RemoteContext";
export function useFusionAvailability() {
  const remote = useRemoteControls();
  const configurable = remote.capabilities.has("fusion_settings_v1");
  const resource = useDesktopResource(remote.getFusionSettings, remote.desktopSettingsRevision, remote.desktopReadable && configurable);
  return remote.capabilities.has("fusion_v1") && (!configurable || resource.data?.enabled === true);
}
