import { decodeJsonBytes as decode } from "@future-os/thread-projection";

/** Small replies use the native parser. Large UTF-8 and graph composition
 * yield between bounded tasks, including on React Native/Hermes. */
export async function decodeJsonBytes<T>(bytes: Uint8Array, current: () => boolean = () => true): Promise<T> {
  return decode<T>(bytes, { isCurrent: current });
}
