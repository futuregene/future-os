import type { FusionSettings } from "@future-os/thread-projection";
import type { RemotePeer } from "./remotePeerClient";
import { useCallback } from "react";
import { useAsyncResource } from "../../lib/useAsyncResource";
import { useHostCatalogueInvalidation } from "./remoteHostCatalog";
import { requestRemotePeer } from "./remotePeerClient";

export function useRemoteFusionAvailability(peer?: RemotePeer) {
  const supported = peer?.features.includes("fusion_v1") === true;
  const configurable = peer?.features.includes("fusion_settings_v1") === true;
  const load = useCallback(() => peer?.connected && configurable
    ? requestRemotePeer<FusionSettings>(peer.desktopId, { type: "get_fusion_settings" }, "list")
    : Promise.resolve(null), [peer?.connected, peer?.desktopId, configurable]);
  const resource = useAsyncResource(load, [load, peer?.bridgeInstanceId], null);
  useHostCatalogueInvalidation(peer?.desktopId ?? "", (domain) => {
    if (domain === "settings")
      resource.reload();
  });
  return supported && (!configurable || (peer?.connected === true && resource.data?.enabled === true));
}
