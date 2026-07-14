/**
 * Unit tests for selector resolver and errors.
 */
import { describe, test, expect } from "bun:test";
import { resolveTarget, parseSelector, UnknownRefError } from "../../selector-resolver.js";
import type { BrowserConfig } from "../../types.js";

const baseConfig: BrowserConfig = {
  version: 2,
  connection: { protocol: "cdp", browserKind: "chrome", endpoint: "http://127.0.0.1:9222" },
  refs: { b1: "#btn-submit", i2: "input[data-testid='email']" },
};

describe("resolveTarget", () => {
  test("ref resolves to stored selector", () => {
    const result = resolveTarget("b1", baseConfig);
    expect(result.source).toBe("ref");
    expect(result.selector).toBe("#btn-submit");
    expect(result.ref).toBe("b1");
  });

  test("ref is case-insensitive", () => {
    const result = resolveTarget("B1", baseConfig);
    expect(result.source).toBe("ref");
    expect(result.selector).toBe("#btn-submit");
  });

  test("unknown ref throws UnknownRefError", () => {
    expect(() => resolveTarget("b99", baseConfig)).toThrow(UnknownRefError);
    try {
      resolveTarget("b99", baseConfig);
    } catch (e) {
      expect((e as Error).message).toContain('"b99"');
      expect((e as Error).message).toContain("snapshot");
    }
  });

  test("selector passes through directly", () => {
    const result = resolveTarget("#my-id", baseConfig);
    expect(result.source).toBe("selector");
    expect(result.selector).toBe("#my-id");
  });

  test("text= selector parsed as text engine", () => {
    const result = resolveTarget("text=Submit", baseConfig);
    expect(result.parsed.engine).toBe("text");
    expect(result.parsed.body).toBe("Submit");
  });

  test("xpath= selector parsed as xpath engine", () => {
    const result = resolveTarget("xpath=//button", baseConfig);
    expect(result.parsed.engine).toBe("xpath");
  });

  test("html selector parsed as css engine", () => {
    const result = resolveTarget(".btn-primary", baseConfig);
    expect(result.parsed.engine).toBe("css");
  });

  test("empty input throws", () => {
    expect(() => resolveTarget("", baseConfig)).toThrow();
    expect(() => resolveTarget(undefined, baseConfig)).toThrow();
  });
});

describe("parseSelector", () => {
  test("standard css is css engine", () => {
    expect(parseSelector("#foo").engine).toBe("css");
    expect(parseSelector(".bar").engine).toBe("css");
    expect(parseSelector("div > span").engine).toBe("css");
  });

  test("text= prefix is text engine", () => {
    expect(parseSelector("text=Click me").engine).toBe("text");
    expect(parseSelector("text=Click me").body).toBe("Click me");
  });

  test("xpath= prefix is xpath engine", () => {
    expect(parseSelector("xpath=//div").engine).toBe("xpath");
    expect(parseSelector("xpath=//div").body).toBe("//div");
  });
});
