import { wsconnect } from "@nats-io/nats-core";
import { RemoteClient as Client, type RemoteClientCallbacks } from "../src/client";
import type { RemoteCredentials } from "../src/types";
import { randomId } from "../src/codec";
import * as host from "./host";
/** Explicit in-memory host, shared only by the package test drivers. */
export class RemoteClient extends Client {
  constructor(credentials: RemoteCredentials, callbacks: RemoteClientCallbacks) {
    super(credentials, callbacks, {
      deviceName: host.deviceName, deviceKind: () => "mobile", randomId,
      refreshCredentials: host.refreshCredentials, connect: wsconnect,
    });
  }
}
