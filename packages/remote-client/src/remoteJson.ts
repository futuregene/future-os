import { remoteJsonBytes } from "@future-os/remote-protocol";
import { decodeJsonBytes } from "./cooperativeJson";
export { decodeRemoteJson } from "@future-os/remote-protocol";

export async function decodeRemoteJsonAsync<T>(data: Uint8Array, current?: () => boolean): Promise<T> {
  return decodeJsonBytes<T>(remoteJsonBytes(data), current);
}
