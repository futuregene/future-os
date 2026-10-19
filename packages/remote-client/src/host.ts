import type { ConnectionOptions, NatsConnection } from "@nats-io/nats-core";
import type { RemoteCredentials } from "./types";

/** Platform services are explicit per client. No UI, environment, secure-store
 * or device-global lookup is performed by the role implementation. */
export interface ClientHost {
  deviceName(): string;
  deviceKind(): string;
  randomId(prefix: string): string;
  refreshCredentials(credentials: RemoteCredentials, signal?: AbortSignal): Promise<RemoteCredentials>;
  /** The host chooses the actual transport; normal Mobile connections use WSS. */
  connect(options: ConnectionOptions): Promise<NatsConnection>;
}
