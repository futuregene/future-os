import { describe, expect, test } from "bun:test";
import { responseData, streamEventData } from "../decode.js";

describe("responseData", () => {
  test("parses a JSON data string", () => {
    const resp = { data: '{"sessionId":"s1","model":"m"}' };
    expect(responseData(resp)).toEqual({ sessionId: "s1", model: "m" });
  });

  test("returns non-string data unchanged", () => {
    const obj = { sessionId: "s1" };
    expect(responseData({ data: obj })).toBe(obj);
  });

  test("returns an unparseable data string as-is", () => {
    expect(responseData({ data: "not json" })).toBe("not json");
  });

  test("empty-string data with no payload stays empty (client parity)", () => {
    expect(responseData({ data: "" })).toBe("");
  });

  test("falls back to the typed oneof when data is undefined", () => {
    // proto-loader oneofs:true exposes the chosen member on `kind`.
    const resp = { payload: { kind: "getState", getState: { sessionId: "s1" } } };
    expect(responseData(resp)).toEqual({ sessionId: "s1" });
  });

  test("typed fallback fires on empty-string data (real loader defaults:true)", () => {
    // proto-loader with defaults:true materializes an ABSENT `data` as "" (not
    // undefined). The typed fallback must fire on "" too, otherwise TUI/CLI
    // would read "" once the agent stops dual-writing.
    const resp = { data: "", payload: { kind: "getState", getState: { sessionId: "s1" } } };
    expect(responseData(resp)).toEqual({ sessionId: "s1" });
  });

  test("returns undefined when neither data nor payload is present", () => {
    expect(responseData({})).toBeUndefined();
  });
});

describe("streamEventData", () => {
  test("parses data and drops the injected type key", () => {
    const ev = { data: '{"type":"text_chunk","text":"hi"}' };
    expect(streamEventData(ev)).toEqual({ text: "hi" });
  });

  test("empty data with no payload yields an empty fields object", () => {
    expect(streamEventData({ data: "" })).toEqual({});
  });

  test("falls back to the typed oneof when data is undefined", () => {
    const ev = { payload: { kind: "toolEnd", toolEnd: { tool_id: "c1", text: "ok" } } };
    expect(streamEventData(ev)).toEqual({ tool_id: "c1", text: "ok" });
  });

  test("typed fallback fires on empty-string data (real loader defaults:true)", () => {
    // proto-loader with defaults:true materializes an ABSENT `data` as "".
    const ev = { data: "", payload: { kind: "textChunk", textChunk: { text: "hi" } } };
    expect(streamEventData(ev)).toEqual({ text: "hi" });
  });

  test("non-object data yields an empty fields object", () => {
    expect(streamEventData({ data: "42" })).toEqual({});
  });
});
