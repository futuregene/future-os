// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Font size module under test.
 *
 * Like the theme, the module keeps its state at module scope (it has to apply
 * before the first React render), so each test re-imports a fresh copy after
 * arranging storage.
 */

beforeEach(() => {
  localStorage.clear();
  document.documentElement.style.removeProperty("--app-font-scale");
  vi.resetModules();
});

describe("font size", () => {
  it("defaults to medium and applies its scale on load", async () => {
    const fontSize = await import("./fontSize");

    expect(fontSize.getFontSize()).toBe("medium");
    expect(fontSize.FONT_SIZE_SCALE.medium).toBe(1);
    expect(document.documentElement.style.getPropertyValue("--app-font-scale")).toBe("1");
  });

  it("restores a stored choice instead of the default", async () => {
    localStorage.setItem("future.fontSize", "large");
    const fontSize = await import("./fontSize");

    expect(fontSize.getFontSize()).toBe("large");
    expect(document.documentElement.style.getPropertyValue("--app-font-scale")).toBe("1.15");
  });

  it.each(["small", "medium", "large", "xlarge"] as const)("switches to %s and persists it", async (size) => {
    const fontSize = await import("./fontSize");

    fontSize.setFontSize(size);

    expect(fontSize.getFontSize()).toBe(size);
    expect(localStorage.getItem("future.fontSize")).toBe(size);
    expect(document.documentElement.style.getPropertyValue("--app-font-scale")).toBe(String(fontSize.FONT_SIZE_SCALE[size]));
  });

  it("ignores a corrupt stored value", async () => {
    localStorage.setItem("future.fontSize", "gigantic");
    const fontSize = await import("./fontSize");

    expect(fontSize.getFontSize()).toBe("medium");
  });

  it("still applies a choice when storage is unavailable", async () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota exceeded");
    });
    const fontSize = await import("./fontSize");

    fontSize.setFontSize("xlarge");

    expect(fontSize.getFontSize()).toBe("xlarge");
    expect(document.documentElement.style.getPropertyValue("--app-font-scale")).toBe("1.35");
    setItem.mockRestore();
  });

  it("keeps working with no document to paint", async () => {
    // The module is imported from code that may run before a DOM exists (and in a
    // non-DOM runner): the choice still has to be read and stored, only the paint
    // is skipped.
    const fontSize = await import("./fontSize");
    vi.stubGlobal("document", undefined);
    try {
      expect(() => fontSize.setFontSize("large")).not.toThrow();
    }
    finally {
      vi.unstubAllGlobals();
    }
    // The choice is remembered even though nothing could be painted.
    expect(fontSize.getFontSize()).toBe("large");
    expect(localStorage.getItem("future.fontSize")).toBe("large");
  });

  it("notifies subscribers on a change and stops after unsubscribe", async () => {
    const fontSize = await import("./fontSize");
    const listener = vi.fn();
    const unsubscribe = fontSize.subscribeFontSize(listener);

    fontSize.setFontSize("small");
    expect(listener).toHaveBeenCalledTimes(1);

    unsubscribe();
    fontSize.setFontSize("large");
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
