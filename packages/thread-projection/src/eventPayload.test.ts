import { describe, expect, test } from "vitest";
import { copyEventPayload, eventPayload, hasEventPayload, withEventPayload } from "./eventPayload";
import { createRunProjector } from "./liveApply";
import { normalizeReplayEvent } from "./replay";

describe("decoded event ownership", () => {
  test("shares the parsed graph across spread, normalization and projection", () => {
    const value = { text: "hydrated body", opaque: { value: [1, 2] } };
    const wire = { type: "text_chunk", data: '{"text":"wire preview"}', runId: "run", idx: 0 };
    const event = withEventPayload(wire, value);
    const normalized = normalizeReplayEvent({ ...event })!;
    expect(eventPayload(normalized, normalized.data)).toBe(value);
    const projected = copyEventPayload(normalized, {
      id: "event", runId: "run", eventType: "text_chunk", payload: normalized.data, sequence: 0, createdAt: 1,
    });
    expect(createRunProjector().ingest([projected]).content).toBe("hydrated body");
    expect(wire.data).toBe('{"text":"wire preview"}');
    expect(JSON.stringify(event)).toBe(JSON.stringify(wire));
    expect(hasEventPayload(wire)).toBe(false);
  });
  test("wire properties cannot impersonate a decoded payload", () => {
    const event = JSON.parse('{"data":"{\\"text\\":\\"wire\\"}","decodedEventPayload":{"text":"forged"},"decodedData":{"text":"forged"}}');
    expect(hasEventPayload(event)).toBe(false);
    expect(eventPayload(event, event.data)).toEqual({ text: "wire" });
  });
  test("does not serialize a decoded body even when serializing its envelope", () => {
    const value = { text: "large", toJSON: () => { throw new Error("body serialized"); } };
    const event = withEventPayload({ data: "{}" }, value);
    expect(eventPayload(event, event.data)).toBe(value);
    expect(JSON.stringify(event)).toBe('{"data":"{}"}');
  });
  test("invalid and absent raw payloads keep the projector's null semantics", () => {
    expect(eventPayload({}, "{")).toBeNull();
    expect(eventPayload({}, null)).toBeNull();
    expect(eventPayload(withEventPayload({}, null), '{"text":"ignored"}')).toBeNull();
  });
});
