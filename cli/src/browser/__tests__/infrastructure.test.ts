/**
 * Smoke test to verify test infrastructure is working.
 */
import { describe, test, expect } from "bun:test";
import { createTestIsolation } from "./isolation.js";
import { getFixture, getFixtureUrl, getFixtureNames, FIXTURE_BASE_URL } from "./fixtures/pages.js";
import { normalizeResult, snapshotConfig, type NormalizerContext } from "./normalizer.js";
import type { BrowserConfig } from "../types.js";

describe("test infrastructure", () => {
  test("all fixture pages load", () => {
    const names = getFixtureNames();
    expect(names.length).toBeGreaterThanOrEqual(10);

    for (const name of names) {
      const html = getFixture(name);
      expect(html).toBeTruthy();
      expect(typeof html).toBe("string");
    }
  });

  test("basic fixture has expected content", () => {
    const html = getFixture("basic");
    expect(html).toContain("Basic Page");
    expect(html).toContain('<button id="btn-submit">');
  });

  test("form fixture has expected content", () => {
    const html = getFixture("form");
    expect(html).toContain("Form Test");
    expect(html).toContain("window.__events");
  });

  test("shadow-dom fixture exists", () => {
    const html = getFixture("shadow-dom");
    expect(html).toContain("Shadow DOM");
    expect(html).toContain("attachShadow");
  });

  test("getFixtureUrl returns valid URL", () => {
    const url = getFixtureUrl("basic");
    expect(url).toBe(`${FIXTURE_BASE_URL}/basic`);
  });

  test("unknown fixture throws", () => {
    expect(() => getFixture("nonexistent")).toThrow("Unknown fixture");
  });

  test("createTestIsolation creates temp structure", async () => {
    const iso = await createTestIsolation();
    expect(iso.tempRoot).toBeTruthy();
    expect(iso.homeDir).toBe(iso.tempRoot + "/home");
    expect(iso.configDir).toContain(".future/agent/browser");

    await iso.cleanup();
  });

  test("normalizer replaces PID in strings and structured content (number)", () => {
    const ctx: NormalizerContext = {
      tempRoot: "/tmp/test-root-12345",
      endpoint: "http://127.0.0.1:9222",
      pid: 45678, // Non-zero PID
    };

    const result = normalizeResult(
      {
        stdout: "Browser running [pid 45678]",
        stderr: "",
        exitCode: 0,
        structuredContent: {
          pid: 45678, // number field
          message: "Process 45678 started", // string field
        },
        text: "OK",
      },
      ctx,
    );

    expect(result.stdout).toBe("Browser running [pid <PID>]");
    expect(result.structuredContent?.pid).toBe("<PID>");
    expect(result.structuredContent?.message).toBe("Process <PID> started");
  });

  test("normalizer replaces temp root and endpoint", () => {
    const ctx: NormalizerContext = {
      tempRoot: "/tmp/test-root-999",
      endpoint: "http://127.0.0.1:9225",
      pid: -1, // Negative PID = no PID replacement
    };

    const result = normalizeResult(
      {
        stdout: "Config at /tmp/test-root-999/config.json via http://127.0.0.1:9225",
        stderr: "",
        exitCode: 0,
        structuredContent: {
          path: "/tmp/test-root-999/screenshot.png",
          endpoint: "http://127.0.0.1:9225",
        },
        text: null,
      },
      ctx,
    );

    expect(result.stdout).toBe("Config at <TEMP_ROOT>/config.json via <ENDPOINT>");
    expect(result.structuredContent?.path).toBe("<TEMP_ROOT>/screenshot.png");
    expect(result.structuredContent?.endpoint).toBe("<ENDPOINT>");
  });

  test("normalizer replaces any localhost endpoint", () => {
    const ctx: NormalizerContext = {
      tempRoot: "/tmp/irrelevant",
      endpoint: "http://127.0.0.1:9999",
      pid: -1,
    };

    // Even though endpoint is :9999, :8080 should also match via regex
    const result = normalizeResult(
      {
        stdout: "Listening at http://127.0.0.1:8080",
        stderr: "",
        exitCode: 0,
        structuredContent: null,
        text: null,
      },
      ctx,
    );

    expect(result.stdout).toBe("Listening at <ENDPOINT>");
  });

  test("snapshotConfig normalizes config fields", () => {
    const config: BrowserConfig = {
      version: 2,
      connection: {
        protocol: "cdp",
        browserKind: "chrome",
        endpoint: "http://127.0.0.1:9225",
      },
      activePageId: "target-abc-123",
      tabOrder: ["target-abc-123", "target-def-456"],
      refs: { b1: "#btn-submit" },
      refsPageId: "target-abc-123",
    };

    const snapped = snapshotConfig(config, {
      tempRoot: "/tmp/test",
      endpoint: "http://127.0.0.1:9225",
      pid: 0,
    });

    const conn = snapped.connection as Record<string, unknown>;
    expect(conn.endpoint).toBe("<ENDPOINT>");
    expect(snapped.activePageId).toBe("<PAGE_ID>");
    expect((snapped.tabOrder as string[])[0]).toBe("<PAGE_ID>");
    expect(snapped.refsPageId).toBe("<PAGE_ID>");
  });
});
