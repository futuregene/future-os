import type { ConnectionOptions, NatsConnection } from "@nats-io/nats-core";
import type { RemoteCredentials } from "./types";

/** Platform services are explicit per client. No UI, environment, secure-store
 * or device-global lookup is performed by the role implementation. */
export interface ClientHost {
  deviceName(): string;
  deviceKind(): string;
  /**
   * The build this client is running, as the platform knows it.
   *
   * Optional because a host that cannot name its own build must say nothing
   * rather than something wrong: the desktop reports an unknown version, which
   * is the same answer an older client gives. It is a label, never a gate —
   * nothing here or on the desktop compares it against a minimum.
   */
  deviceVersion?(): string | null;
  randomId(prefix: string): string;
  refreshCredentials(credentials: RemoteCredentials, signal?: AbortSignal): Promise<RemoteCredentials>;
  /** The host chooses the actual transport; normal Mobile connections use WSS. */
  connect(options: ConnectionOptions): Promise<NatsConnection>;
}
