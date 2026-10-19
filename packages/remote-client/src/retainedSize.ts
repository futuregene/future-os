/** Conservative cache-accounting estimate, not a VM heap measurement. Walk
 * structures without allocating a serialized copy; count Map/Set contents too.
 * Closures require their owner's explicit estimate. Shared objects count once. */
export function retainedSize(value: unknown): number {
  const stack: unknown[] = [value];
  const seen = new Set<object>();
  let bytes = 0;
  while (stack.length) {
    const item = stack.pop();
    if (typeof item === "string") { bytes += 24 + item.length * 2; continue; }
    if (item === null || typeof item !== "object") { bytes += 8; continue; }
    if (seen.has(item)) continue;
    seen.add(item);
    bytes += 64;
    if (item instanceof Map) {
      for (const [key, entry] of item) { bytes += 32; stack.push(key, entry); }
    } else if (item instanceof Set) {
      for (const entry of item) { bytes += 24; stack.push(entry); }
    } else if (Array.isArray(item)) {
      bytes += item.length * 8;
      for (const entry of item) stack.push(entry);
    } else {
      for (const [key, entry] of Object.entries(item)) { bytes += 16 + key.length * 2; stack.push(entry); }
    }
  }
  return bytes;
}
