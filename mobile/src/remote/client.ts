import { RemoteClient as Client, type RemoteClientCallbacks } from "@future-os/remote-client/client";
import { wsconnect } from "@nats-io/nats-core";
import { deviceName, refreshCredentials } from "./pairing";
import { randomId } from "./codec";
import type { RemoteCredentials } from "./types";
export type { RemoteClientCallbacks, RecoveryReason } from "@future-os/remote-client/client";
export { isDeferredRequestError, isTransientNatsRequestError } from "@future-os/remote-client/client";

/** Mobile supplies its identity, control-plane service and WSS transport. */
export class RemoteClient extends Client {
  constructor(credentials: RemoteCredentials, callbacks: RemoteClientCallbacks) {
    super(credentials, callbacks, {
      deviceName, deviceKind: () => "mobile", randomId,
      refreshCredentials, connect: wsconnect,
    });
  }
}
