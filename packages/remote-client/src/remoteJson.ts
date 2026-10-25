import { remoteReplyBody } from "@future-os/remote-protocol";
import { decodeJsonBytes } from "./cooperativeJson";
export { decodeRemoteJson } from "@future-os/remote-protocol";

export async function decodeRemoteJsonAsync<T>(data: Uint8Array, current?: () => boolean): Promise<T> {
  return decodeJsonBytes<T>(remoteReplyBody(data).json, current);
}

/**
 * Decode a command reply, keeping any bytes it carried outside its JSON.
 *
 * The two carriers put the same envelope in different places: a legacy reply is
 * one JSON document, a negotiated page is a JSON header followed by the chunk's
 * raw bytes (see `@future-os/remote-protocol`'s `remoteReplyBody`). `chunkBytes`
 * is where those bytes land — attached non-enumerably so this stays a *plain*
 * reply to every caller that compares or spreads one, and present only for the
 * replies that actually carried bytes.
 */
export async function decodeRemoteReplyAsync<T extends object>(
  data: Uint8Array,
  current?: () => boolean,
): Promise<T> {
  const { json, body } = remoteReplyBody(data);
  const reply = await decodeJsonBytes<T>(json, current);
  if (body) {
    Object.defineProperty(reply, "chunkBytes", {
      value: body, enumerable: false, configurable: true,
    });
  }
  return reply;
}
