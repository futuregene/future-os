const SMALL_JSON = 256 * 1024;
const pause = () => new Promise<void>(resolve => setTimeout(resolve, 0));

/** Keep the native parser: a custom streaming parser measured substantially
 * more CPU/wall time on real traces. Split UTF-8 decoding from graph allocation
 * with a task boundary so navigation can cancel between those native phases.
 * Neither individual native phase is preemptible; this is not a worker. */
export async function decodeJsonBytes<T>(bytes: Uint8Array, current: () => boolean = () => true): Promise<T> {
  const check = () => { if (!current()) throw new Error("stale_json_decode"); };
  check();
  const text = new TextDecoder().decode(bytes);
  if (bytes.length >= SMALL_JSON) { await pause(); check(); }
  return JSON.parse(text) as T;
}
