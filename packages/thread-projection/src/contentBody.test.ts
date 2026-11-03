import { describe, expect, test } from "vitest";
import { createContentEventLane, hydrateRequiredContent, readContentField, readContentPages } from "./contentBody";
import type { ContentPage, ContentRequest } from "./contentBody";
import { eventPayload } from "./eventPayload";
const reference: ContentRequest = { databaseId: "db", restoreGeneration: "generation", epoch: 1, cursor: 2, field: "/data/text" };
function page(text: string, next: number, total: number, more: boolean): ContentPage {
  return { text, nextOffset: next, totalBytes: total, hasMore: more, etag: "immutable", encoding: "text", databaseId: "db", restoreGeneration: "generation" };
}
describe("canonical content pages", () => {
  test("walks exact UTF-8 offsets and yields the original body", async () => {
    const offsets: number[] = [];
    const result = await readContentField(reference, async request => {
      offsets.push(request.offset!);
      return request.offset === 0 ? page("你", 3, 7, true) : page("好a", 7, 7, false);
    });
    expect(result).toBe("你好a");
    expect(offsets).toEqual([0, 3]);
  });
  test("rejects restore changes, byte gaps, changed etags and a stalled page", async () => {
    for (const bad of [
      { ...page("x", 1, 1, false), restoreGeneration: "other" },
      { ...page("你", 1, 3, true) },
      { ...page("", 0, 1, true) },
    ]) await expect(readContentField(reference, async () => bad)).rejects.toThrow();
    let call = 0;
    await expect(readContentField(reference, async () => ++call === 1 ? page("a", 1, 2, true) : { ...page("b", 2, 2, false), etag: "different" })).rejects.toThrow("content_page_changed");
  });
  test("an explicit page iterator supports details above materialization budget", async () => {
    const read = async () => page("x", 1, 100, true);
    await expect(readContentField(reference, read, () => true, JSON.parse, 16)).rejects.toThrow("content_materialization_too_large");
    for await (const result of readContentPages(reference, read)) { expect(result.text).toBe("x"); break; }
  });
  test("loads approval facts but leaves tool bodies lazy and validates field ownership", async () => {
    const source = { text: "", content_refs: { text: reference } };
    expect(await hydrateRequiredContent("tool_end", source, async () => { throw new Error("must stay lazy"); })).toBe(source);
    expect(await hydrateRequiredContent("approval_request", source, async () => page("decision", 8, 8, false))).toEqual({ text: "decision" });
    await expect(hydrateRequiredContent("user_message", { content_refs: { unrelated: reference } }, async () => page("x", 1, 1, false))).rejects.toThrow("content_field_mismatch");
  });
  test("serializes a required read ahead of following events and fences disposal", async () => {
    let release!: (result: ContentPage) => void;
    let started!: () => void;
    let reading = new Promise<void>(resolve => { started = resolve; });
    const delivered: number[] = [];
    const lane = createContentEventLane({
      read: () => new Promise<ContentPage>(resolve => { release = resolve; started(); }),
      deliver: (event: { type: string; data: string; id: number }) => delivered.push(event.id),
      recover: (error) => { throw error; },
    });
    lane.enqueue({ type: "user_message", data: JSON.stringify({ text: "", content_refs: { text: reference } }), id: 1 });
    lane.enqueue({ type: "text_chunk", data: '{"text":"later"}', id: 2 });
    expect(delivered).toEqual([]);
    await reading;
    release(page("a", 1, 1, false));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(delivered).toEqual([1, 2]);
    reading = new Promise<void>(resolve => { started = resolve; });
    lane.enqueue({ type: "user_message", data: JSON.stringify({ content_refs: { text: reference } }), id: 3 });
    await reading;
    lane.dispose();
    release(page("b", 1, 1, false));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(delivered).toEqual([1, 2]);
  });
  test("delivers a decoded body to the projector without serializing it back into wire data", async () => {
    const wire = { type: "user_message", data: JSON.stringify({ text: "", content_refs: { text: reference } }) };
    let resolve!: (event: typeof wire) => void;
    const delivered = new Promise<typeof wire>(done => { resolve = done; });
    const lane = createContentEventLane({
      read: async () => page("hydrated", 8, 8, false),
      deliver: resolve,
      recover: error => { throw error; },
    });
    lane.enqueue(wire);
    const event = await delivered;
    expect(event.data).toBe(wire.data);
    expect(eventPayload(event, event.data)).toEqual({ text: "hydrated" });
    lane.dispose();
  });
  test("limits cumulative required body size before materializing another field", async () => {
    const source = { content_refs: { text: reference, error: { ...reference, field: "/data/error" } } };
    let reads = 0;
    await expect(hydrateRequiredContent("error", source, async () => {
      reads += 1;
      return reads === 1 ? page("first!", 6, 6, false) : page("next!", 5, 5, false);
    }, () => true, 10)).rejects.toThrow("content_materialization_too_large");
    expect(reads).toBe(2);
  });
});
