import { RemoteClient as Client, type RemoteClientCallbacks } from "@future-os/remote-client/client";
import { wsconnect } from "@nats-io/nats-core";
import { deviceName, deviceVersion, refreshCredentials } from "./pairing";
import { randomId } from "./codec";
import type { RemoteCredentials } from "./types";
export type { RemoteClientCallbacks, RecoveryReason } from "@future-os/remote-client/client";
export { isDeferredRequestError, isTransientNatsRequestError } from "@future-os/remote-client/client";

/** Mobile supplies its identity, control-plane service and WSS transport. */
export class RemoteClient extends Client {
  // The phone inherits the presence promise from `Client`: it declares
  // `client_presence_v1` on `secure_ready`, beats every 15 s while connected,
  // says `background` when the app is suspended (`setAppActive(false)`) and
  // `disconnected` when the user stops it (`shutdown`). A phone that is force-
  // quit cannot say anything, which is the host's lease to notice — that is why
  // the beats exist at all, and why a subclass that overrode
  // `declaredFeatures` would have to start them too.
  constructor(credentials: RemoteCredentials, callbacks: RemoteClientCallbacks) {
    super(credentials, callbacks, {
      deviceName, deviceKind: () => "mobile", deviceVersion, randomId,
      refreshCredentials, connect: wsconnect,
    });
  }
}
