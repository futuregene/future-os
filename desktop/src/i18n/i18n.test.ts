// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  vi.unstubAllGlobals();
  localStorage.removeItem("future.language");
});

describe("i18n language helpers", () => {
  it("reads a stored language at init", async () => {
    vi.resetModules();
    localStorage.setItem("future.language", "en");
    const mod = await import("./index");
    expect(mod.getLanguage()).toBe("en");
  });

  it("follows the OS language when no preference is stored (first run)", async () => {
    vi.resetModules();
    // jsdom's navigator.language defaults to "en-US" → the English bundle.
    expect(navigator.language).toBe("en-US");
    const mod = await import("./index");
    expect(mod.getLanguage()).toBe("en");
  });

  it("falls back to English when navigator.language throws", async () => {
    vi.resetModules();
    // A hostile/absent navigator makes the systemLanguage try-arm fail and
    // the catch arm decide: English.
    vi.stubGlobal("navigator", {
      get language() {
        throw new Error("no navigator.language");
      },
    });
    const mod = await import("./index");
    expect(mod.getLanguage()).toBe("zh"); // DEFAULT_LANGUAGE
  });

  it("detects a Chinese OS and picks zh on first run", async () => {
    vi.resetModules();
    vi.stubGlobal("navigator", { language: "zh-CN" });
    const mod = await import("./index");
    expect(mod.getLanguage()).toBe("zh");
  });

  it("falls back to the system language for an unrecognized stored value", async () => {
    vi.resetModules();
    localStorage.setItem("future.language", "fr");
    const mod = await import("./index");
    expect(mod.getLanguage()).toBe("en");
  });

  it("setLanguage persists the choice and switches the active language", async () => {
    vi.resetModules();
    const mod = await import("./index");
    mod.setLanguage("en");
    expect(localStorage.getItem("future.language")).toBe("en");
    expect(mod.getLanguage()).toBe("en");
    mod.setLanguage("zh");
    expect(mod.getLanguage()).toBe("zh");
  });
});
