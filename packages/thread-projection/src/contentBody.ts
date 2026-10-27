/** Canonical event fields are authorized by the session/run command scope and
 * an immutable stream coordinate. A raw content object ID is never accepted. */
import { parseJsonText } from "./cooperativeJson";
import { hasEventPayload, withEventPayload } from "./eventPayload";
export interface ContentRequest {
  databaseId: string;
  restoreGeneration: string;
  epoch: number;
  cursor: number;
  field: string;
  offset?: number;
  maxBytes?: number;
}
export interface ContentPage {
  text: string;
  nextOffset: number;
  totalBytes: number;
  hasMore: boolean;
  encoding: "text" | "json";
  etag: string;
  databaseId: string;
  restoreGeneration: string;
}
export type ContentReader = (request: ContentRequest) => Promise<ContentPage>;
const PAGE_BYTES = 64 * 1024;
const encoder = new TextEncoder();
const pause = () => new Promise<void>(resolve => setTimeout(resolve, 0));
function current(check: () => boolean): void {
  if (!check()) throw new Error("stale_content_read");
}
function requestOf(value: unknown): ContentRequest {
  if (!value || typeof value !== "object") throw new Error("invalid_content_reference");
  const r = value as Partial<ContentRequest>;
  if (typeof r.databaseId !== "string" || !r.databaseId || typeof r.restoreGeneration !== "string"
    || !r.restoreGeneration || !Number.isSafeInteger(r.epoch) || r.epoch! < -1
    || !Number.isSafeInteger(r.cursor) || r.cursor! < 0 || typeof r.field !== "string"
    || !r.field.startsWith("/data/") || r.field.length > 512) throw new Error("invalid_content_reference");
  return { ...r, offset: 0, maxBytes: PAGE_BYTES } as ContentRequest;
}

/** A detail view can consume pages without allocating the complete body. */
export async function* readContentPages(
  reference: unknown, read: ContentReader, isCurrent: () => boolean = () => true,
): AsyncGenerator<ContentPage> {
  const request = requestOf(reference);
  let offset = 0;
  let stamp: string | undefined;
  for (;;) {
    current(isCurrent);
    const page = await read({ ...request, offset });
    current(isCurrent);
    const bytes = typeof page.text === "string" ? encoder.encode(page.text).byteLength : -1;
    if (page.databaseId !== request.databaseId || page.restoreGeneration !== request.restoreGeneration
      || !Number.isSafeInteger(page.nextOffset) || !Number.isSafeInteger(page.totalBytes) || page.totalBytes < 0
      || page.nextOffset !== offset + bytes || bytes > PAGE_BYTES || bytes < 0 || page.nextOffset > page.totalBytes
      || typeof page.hasMore !== "boolean" || page.hasMore !== (page.nextOffset < page.totalBytes)
      || (page.nextOffset <= offset && page.nextOffset !== page.totalBytes)
      || typeof page.etag !== "string" || !page.etag || !["text", "json"].includes(page.encoding))
      throw new Error("invalid_content_page");
    const nextStamp = JSON.stringify([page.etag, page.totalBytes, page.encoding]);
    if (stamp !== undefined && stamp !== nextStamp) throw new Error("content_page_changed");
    stamp = nextStamp;
    yield page;
    offset = page.nextOffset;
    if (!page.hasMore) return;
    await pause();
  }
}

/** Materialization is for explicit details and essential semantic events only.
 * The caller may provide a worker JSON decoder; normal token rendering never
 * enters this path. Oversized bodies remain available through page iteration. */
export async function readContentField(
  reference: unknown, read: ContentReader, isCurrent: () => boolean = () => true,
  decode?: (text: string) => unknown | Promise<unknown>,
  maxBytes = 16 * 1024 * 1024,
): Promise<unknown> {
  const parts: string[] = [];
  let encoding: ContentPage["encoding"] = "text";
  for await (const page of readContentPages(reference, read, isCurrent)) {
    if (page.totalBytes > maxBytes) throw new Error("content_materialization_too_large");
    parts.push(page.text);
    encoding = page.encoding;
  }
  current(isCurrent);
  const text = parts.join("");
  if (encoding === "text") return text;
  await pause();
  current(isCurrent);
  const result = await (decode ? decode(text) : parseJsonText(text, { isCurrent }));
  current(isCurrent);
  return result;
}
export function needsContentHydration(kind: string, data: unknown): boolean {
  return ["user_message", "approval_request", "error", "agent_end", "compaction_committed", "compaction_failed"].includes(kind)
    && !!data && typeof data === "object" && "content_refs" in data
    && !!data.content_refs && typeof data.content_refs === "object";
}
export async function hydrateRequiredContent(
  kind: string, data: Record<string, unknown>, read: ContentReader, isCurrent: () => boolean = () => true,
  maxBytes = 16 * 1024 * 1024,
): Promise<Record<string, unknown>> {
  if (!needsContentHydration(kind, data)) return data;
  const refs = data.content_refs as Record<string, unknown>;
  if (Object.keys(refs).length > 128) throw new Error("content_field_count_exceeded");
  if (!Number.isSafeInteger(maxBytes) || maxBytes < 0 || maxBytes > 16 * 1024 * 1024)
    throw new Error("invalid_content_materialization_budget");
  let remaining = maxBytes;
  const hydrated = { ...data };
  for (const [name, reference] of Object.entries(refs)) {
    const request = requestOf(reference);
    if (request.field !== `/data/${name.replace(/~/g, "~0").replace(/\//g, "~1")}`)
      throw new Error("content_field_mismatch");
    const value = await readContentField(request, async range => {
      const page = await read(range);
      if (range.offset === 0) {
        if (!Number.isSafeInteger(page.totalBytes) || page.totalBytes < 0 || page.totalBytes > remaining)
          throw new Error("content_materialization_too_large");
        remaining -= page.totalBytes;
      }
      return page;
    }, isCurrent, undefined, remaining);
    Object.defineProperty(hydrated, name, { value,
      enumerable: true, writable: true, configurable: true });
  }
  delete hydrated.content_refs;
  return hydrated;
}

/** Preserve event order while a required body is loading. The common inline
 * path is synchronous. A failed/overfull lane requests durable replay before
 * accepting further state as complete; stale in-flight reads cannot deliver. */
export function createContentEventLane<T extends { type: string; data: string; runId?: string }>(options: {
  read(event: T, request: ContentRequest): Promise<ContentPage>;
  deliver(event: T): void;
  recover(error: unknown): void;
  isCurrent?: () => boolean;
  maxQueuedBytes?: number;
}) {
  let queue: Array<{ event: T; bytes: number }> = [];
  let bytes = 0;
  let running = false;
  let disposed = false;
  let version = 0;
  const valid = () => !disposed && (options.isCurrent?.() ?? true);
  const fail = (error: unknown) => {
    version += 1;
    queue = [];
    bytes = 0;
    if (valid()) options.recover(error);
  };
  async function pump() {
    if (running || !valid()) return;
    running = true;
    let count = 0;
    try {
      while (queue.length && valid()) {
        const next = queue.shift()!;
        bytes -= next.bytes;
        const captured = version;
        const currentRead = () => valid() && captured === version;
        let event = next.event;
        if (!hasEventPayload(event) && (event.data.includes('"content_refs"') || event.data.length > 32 * 1024)) {
          const data = await parseJsonText<Record<string, unknown>>(event.data, { isCurrent: currentRead });
          try {
            const hydrated = await hydrateRequiredContent(event.type, data, request => options.read(event, request), currentRead);
            if (!currentRead()) continue;
            event = withEventPayload(event, hydrated);
          } catch (error) {
            if (!currentRead()) continue;
            throw error;
          }
        }
        if (currentRead()) options.deliver(event);
        if (++count >= 64) { count = 0; await pause(); }
      }
    } catch (error) { fail(error); }
    finally {
      running = false;
      if (queue.length && valid()) void pump();
    }
  }
  return {
    enqueue(event: T) {
      if (!valid()) return;
      if (!running && queue.length === 0 && (hasEventPayload(event) || (event.data.length <= 32 * 1024 && !event.data.includes('"content_refs"')))) {
        options.deliver(event);
        return;
      }
      const size = event.data.length * 2 + 512;
      if (queue.length >= 4096 || bytes + size > (options.maxQueuedBytes ?? 8 * 1024 * 1024)) {
        fail(new Error("content_event_lane_overflow"));
        return;
      }
      queue.push({ event, bytes: size });
      bytes += size;
      void pump();
    },
    dispose() { disposed = true; version += 1; queue = []; bytes = 0; },
  };
}
