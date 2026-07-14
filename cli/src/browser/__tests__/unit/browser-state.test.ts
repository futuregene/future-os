/**
 * Unit tests for BrowserConfig parse/migrate/validate.
 */
import { describe, test, expect } from "bun:test";
import { parseBrowserConfig, InvalidBrowserConfigError } from "../../browser-state.js";

describe("parseBrowserConfig", () => {
  test("empty object → default v2 config", () => {
    const config = parseBrowserConfig({});
    expect(config.version).toBe(2);
    expect(config.connection.protocol).toBe("cdp");
    expect(config.connection.browserKind).toBe("chromium");
    expect(config.connection.endpoint).toBe("http://127.0.0.1:9222");
  });

  test("version=1 → migrated to v2", () => {
    const config = parseBrowserConfig({ version: 1, endpoint: "http://127.0.0.1:9225" });
    expect(config.version).toBe(2);
    expect(config.connection.protocol).toBe("cdp");
    expect(config.connection.browserKind).toBe("chromium");
    expect(config.connection.endpoint).toBe("http://127.0.0.1:9225");
  });

  test("v1 without endpoint → default endpoint", () => {
    const config = parseBrowserConfig({ version: 1 });
    expect(config.connection.endpoint).toBe("http://127.0.0.1:9222");
  });

  test("v1 invalid endpoint throws", () => {
    expect(() => parseBrowserConfig({ version: 1, endpoint: "not-a-url" }))
      .toThrow(InvalidBrowserConfigError);
  });

  test("valid v2 CDP config passes", () => {
    const config = parseBrowserConfig({
      version: 2,
      connection: {
        protocol: "cdp",
        browserKind: "chrome",
        endpoint: "http://127.0.0.1:9222",
      },
    });
    expect(config.version).toBe(2);
    expect(config.connection.protocol).toBe("cdp");
    expect(config.connection.browserKind).toBe("chrome");
  });

  test("v2 with activePageId and tabOrder preserved", () => {
    const config = parseBrowserConfig({
      version: 2,
      connection: { protocol: "cdp", browserKind: "edge", endpoint: "http://127.0.0.1:9999" },
      activePageId: "target-123",
      tabOrder: ["target-123", "target-456"],
      refs: { b1: "#btn" },
      refsPageId: "target-123",
    });
    expect(config.activePageId).toBe("target-123");
    expect(config.tabOrder).toEqual(["target-123", "target-456"]);
    expect(config.refs).toEqual({ b1: "#btn" });
  });

  test("future version throws", () => {
    expect(() => parseBrowserConfig({ version: 99 })).toThrow(/unsupported/i);
  });

  test("version=0 throws", () => {
    expect(() => parseBrowserConfig({ version: 0 })).toThrow(/unsupported/i);
  });

  test("version as string throws", () => {
    expect(() => parseBrowserConfig({ version: "2" })).toThrow(/unsupported/i);
  });

  test("v2 without connection throws", () => {
    expect(() => parseBrowserConfig({ version: 2 })).toThrow(InvalidBrowserConfigError);
  });

  test("v2 with invalid protocol throws", () => {
    expect(() => parseBrowserConfig({
      version: 2,
      connection: { protocol: "banana", browserKind: "chrome", endpoint: "http://x" },
    })).toThrow(/protocol/i);
  });

  test("v2 CDP with safari browserKind throws", () => {
    expect(() => parseBrowserConfig({
      version: 2,
      connection: { protocol: "cdp", browserKind: "safari", endpoint: "http://x" },
    })).toThrow(/browser kind/i);
  });

  test("v2 webdriver with invalid browserKind throws", () => {
    expect(() => parseBrowserConfig({
      version: 2,
      connection: { protocol: "webdriver", browserKind: "chrome", endpoint: "http://x", sessionId: "s1" },
    })).toThrow(/browser kind/i);
  });

  test("v2 webdriver valid config passes", () => {
    const config = parseBrowserConfig({
      version: 2,
      connection: {
        protocol: "webdriver",
        browserKind: "safari",
        endpoint: "http://127.0.0.1:4444",
        sessionId: "abc-123",
        driverPid: 45678,
      },
    });
    expect(config.connection.protocol).toBe("webdriver");
    if (config.connection.protocol === "webdriver") {
      expect(config.connection.sessionId).toBe("abc-123");
      expect(config.connection.driverPid).toBe(45678);
    }
  });

  test("v2 Safari config missing sessionId recovers to CDP defaults", () => {
    const config = parseBrowserConfig({
      version: 2,
      connection: {
        protocol: "webdriver",
        browserKind: "safari",
        endpoint: "http://127.0.0.1:4444",
        driverPid: 45678,
      },
      activePageId: "stale-safari-window",
    });
    expect(config).toEqual({
      version: 2,
      connection: {
        protocol: "cdp",
        browserKind: "chromium",
        endpoint: "http://127.0.0.1:9222",
      },
    });
  });

  test("v2 Safari config with blank sessionId still throws", () => {
    expect(() => parseBrowserConfig({
      version: 2,
      connection: {
        protocol: "webdriver",
        browserKind: "safari",
        endpoint: "http://127.0.0.1:4444",
        sessionId: "  ",
      },
    })).toThrow(/sessionId/i);
  });

  test("v2 missing endpoint throws", () => {
    expect(() => parseBrowserConfig({
      version: 2,
      connection: { protocol: "cdp", browserKind: "chrome" },
    })).toThrow(/endpoint/i);
  });

  test("v2 endpoint not http(s) throws", () => {
    expect(() => parseBrowserConfig({
      version: 2,
      connection: { protocol: "cdp", browserKind: "chrome", endpoint: "ftp://bad" },
    })).toThrow(/http/i);
  });

  test("v2 tabOrder not array throws", () => {
    expect(() => parseBrowserConfig({
      version: 2,
      connection: { protocol: "cdp", browserKind: "chrome", endpoint: "http://x" },
      tabOrder: "not-an-array",
    })).toThrow(/tabOrder/i);
  });

  test("v2 refs with non-string values throws", () => {
    expect(() => parseBrowserConfig({
      version: 2,
      connection: { protocol: "cdp", browserKind: "chrome", endpoint: "http://x" },
      refs: { b1: 123 },
    })).toThrow(/refs/i);
  });

  test("not an object throws", () => {
    expect(() => parseBrowserConfig(null)).toThrow();
    expect(() => parseBrowserConfig("invalid")).toThrow();
    expect(() => parseBrowserConfig([])).toThrow();
  });
});
