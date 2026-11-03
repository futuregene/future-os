import { describe, expect, test } from "vitest";
import { decodeJsonBytes, parseJsonText, type JsonDecodeOptions } from "./cooperativeJson";

function scheduler() {
  let clock = 0;
  let yields = 0;
  const options: JsonDecodeOptions = {
    nativeLimit: 0,
    now: () => ++clock,
    sliceMs: 1,
    yieldTask: async () => { yields += 1; },
  };
  return { options, yields: () => yields };
}

describe("cooperative JSON composition", () => {
  test.each([
    '{"__proto__":{"polluted":true},"constructor":1,"a":0,"a":2}',
    '[null,true,false,-0,1e400,-1.25e-3,{},[]]',
    JSON.stringify({ text: `${"x".repeat(8190)}🙂中\\\"\n${"y".repeat(16384)}` }),
    `"${"x".repeat(8191)}\\uD83D\\uDE42\\uD800\\uDC00"`,
    JSON.stringify(Array.from({ length: 10000 }, (_, index) => ({ index, value: [index, null] }))),
  ])("preserves native values and own keys", async source => {
    const schedule = scheduler();
    const result = await parseJsonText(source, schedule.options);
    expect(result).toEqual(JSON.parse(source));
    expect(Object.getPrototypeOf(result as object)).toBe(Object.getPrototypeOf(JSON.parse(source)));
    expect(({} as Record<string, unknown>).polluted).toBeUndefined();
  });

  test.each(['[1,]', '{"a":1,}', '[01]', '[1 2]', '{"a" 2}', '{', 'true false', '"bad\\x"', '"bad\\u00"', '"a\nb"', '', '[', '[true:false]'])
  ("rejects invalid grammar: %s", async source => {
    await expect(parseJsonText(source, scheduler().options)).rejects.toBeInstanceOf(SyntaxError);
  });

  test("yields within large strings, whitespace and containers", async () => {
    for (const source of [JSON.stringify("x".repeat(100000)), `${" ".repeat(100000)}null`, JSON.stringify(Array.from({ length: 10000 }, (_, index) => index))]) {
      const schedule = scheduler();
      expect(await parseJsonText(source, schedule.options)).toEqual(JSON.parse(source));
      expect(schedule.yields()).toBeGreaterThan(2);
    }
  });

  test("cancels between slices and releases its admission", async () => {
    let current = true;
    const schedule = scheduler();
    await expect(parseJsonText(JSON.stringify("x".repeat(100000)), {
      ...schedule.options,
      isCurrent: () => current,
      yieldTask: async () => { current = false; },
    })).rejects.toThrow("stale_json_decode");
    expect(await parseJsonText("[1]", schedule.options)).toEqual([1]);
  });

  test("decodes UTF-8 across byte boundaries and accepts the native BOM convention", async () => {
    const source = JSON.stringify({ text: `${"x".repeat(65524)}🙂中${"y".repeat(100000)}` });
    const bytes = new TextEncoder().encode(`\uFEFF${source}`);
    const schedule = scheduler();
    expect(await decodeJsonBytes(bytes, schedule.options)).toEqual(JSON.parse(source));
    expect(schedule.yields()).toBeGreaterThan(2);
  });

  test("refuses unsupported depth and oversized native literals explicitly", async () => {
    await expect(parseJsonText(`${"[".repeat(129)}0${"]".repeat(129)}`, scheduler().options)).rejects.toThrow("json_decode_depth_exceeded");
    await expect(parseJsonText("1".repeat(8193), scheduler().options)).rejects.toThrow("json_decode_literal_too_large");
  });
});
