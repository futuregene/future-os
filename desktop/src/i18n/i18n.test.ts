// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";

describe("i18n language helpers", () => {
  it("reads a stored language at init", async () => {
    vi.resetModules();
    localStorage.setItem("future.language", "en");
    const mod = await import("./index");
    expect(mod.getLanguage()).toBe("en");
  });

  it("falls back to the default for an unrecognized stored value", async () => {
    vi.resetModules();
    localStorage.setItem("future.language", "fr");
    const mod = await import("./index");
    expect(mod.getLanguage()).toBe("zh");
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
