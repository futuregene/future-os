import { isRecord } from "./utils";

/** Content offsets count UTF-8 bytes, independently of token/event boundaries. */
export function applyContentRange(payload: unknown, eventType: string, offsets: Map<string, number>): unknown {
  if (!isRecord(payload) || typeof payload.content_key !== "string") return payload;
  const { content_key: key, content_offset: offset, content_bytes: length } = payload;
  if (typeof offset !== "number" || !Number.isSafeInteger(offset) || offset < 0
    || typeof length !== "number" || !Number.isSafeInteger(length) || length < 0
    || !Number.isSafeInteger(offset + length)) throw new Error("Invalid canonical content range");
  const seen = offsets.get(key) ?? 0;
  if (offset > seen && payload.content_seal !== true) throw new Error("Canonical content has a missing prefix");
  let result = payload;
  if (["text_chunk", "thinking_delta", "tool_delta"].includes(eventType)) {
    const text = typeof payload.text === "string" ? payload.text : "";
    // Walk only this bounded fragment. Never encode the growing reply to find
    // a byte offset (React Native also needs no TextEncoder dependency).
    const overlap = Math.min(Math.max(seen - offset, 0), length);
    let bytes = 0;
    let units = 0;
    let trim = overlap === 0 ? 0 : -1;
    for (const char of text) {
      const point = char.codePointAt(0)!;
      bytes += point <= 0x7F ? 1 : point <= 0x7FF ? 2 : point <= 0xFFFF ? 3 : 4;
      units += char.length;
      if (bytes === overlap) trim = units;
    }
    if (bytes !== length || trim < 0) throw new Error("Canonical content length or UTF-8 boundary mismatch");
    result = { ...payload, text: text.slice(trim), content_offset: offset + overlap, content_bytes: length - overlap };
  }
  if (payload.content_seal === true) offsets.delete(key);
  else offsets.set(key, Math.max(seen, offset + length));
  return result;
}

/** Source cursor coverage, used before applying the body range. */
export function contentRangeStart(data: string | undefined): number | undefined {
  // Canonical envelopes have a 64 KiB wire limit. Legacy bodies may be much
  // larger; cursor validation must never parse their complete body on the UI
  // thread, nor accept an oversized envelope as canonical overlap authority.
  if (!data || data.length > 64 * 1024 || !data.includes('"content_key"')) return undefined;
  try {
    const value: unknown = JSON.parse(data);
    if (!isRecord(value) || typeof value.content_key !== "string"
      || typeof value.content_first_idx !== "number" || !Number.isSafeInteger(value.content_first_idx) || value.content_first_idx < 0
      || typeof value.content_offset !== "number" || !Number.isSafeInteger(value.content_offset) || value.content_offset < 0
      || typeof value.content_bytes !== "number" || !Number.isSafeInteger(value.content_bytes) || value.content_bytes < 0) return undefined;
    return value.content_first_idx;
  } catch { return undefined; }
}
