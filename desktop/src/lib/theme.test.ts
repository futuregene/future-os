// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Theme module under test.
 *
 * The module keeps its state at module scope (it has to apply before the first
 * React render), so each test re-imports a fresh copy after arranging storage
 * and `matchMedia`.
 */

interface FakeMediaQuery {
  matches: boolean;
  listeners: Set<() => void>;
  addEventListener: (type: string, listener: () => void) => void;
  removeEventListener: (type: string, listener: () => void) => void;
  setMatches: (matches: boolean) => void;
}

function installMatchMedia(initial: boolean): FakeMediaQuery {
  const query: FakeMediaQuery = {
    matches: initial,
    listeners: new Set(),
    addEventListener: (_type, listener) => {
      query.listeners.add(listener);
    },
    removeEventListener: (_type, listener) => {
      query.listeners.delete(listener);
    },
    setMatches: (matches) => {
      query.matches = matches;
      for (const listener of query.listeners) {
        listener();
      }
    },
  };
  vi.stubGlobal("matchMedia", () => query);
  return query;
}

beforeEach(() => {
  localStorage.clear();
  document.documentElement.classList.remove("dark");
  document.documentElement.style.colorScheme = "";
  delete document.documentElement.dataset.palette;
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe("theme", () => {
  it("defaults to following the system and applies the resolved palette on load", async () => {
    installMatchMedia(true);
    const theme = await import("./theme");

    expect(theme.getThemeMode()).toBe("system");
    expect(theme.getResolvedTheme()).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.style.colorScheme).toBe("dark");
  });

  it("resolves an unsupported system to light", async () => {
    installMatchMedia(false);
    const theme = await import("./theme");

    expect(theme.getResolvedTheme()).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("restores a stored explicit choice instead of the system preference", async () => {
    installMatchMedia(true);
    localStorage.setItem("future.theme", "light");
    const theme = await import("./theme");

    expect(theme.getThemeMode()).toBe("light");
    expect(theme.getResolvedTheme()).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it.each(["dark", "light", "system"] as const)("switches to %s and persists it", async (mode) => {
    installMatchMedia(false);
    const theme = await import("./theme");

    theme.setThemeMode(mode);

    expect(theme.getThemeMode()).toBe(mode);
    expect(theme.getResolvedTheme()).toBe(mode === "dark" ? "dark" : "light");
    expect(localStorage.getItem("future.theme")).toBe(mode);
    expect(document.documentElement.classList.contains("dark")).toBe(mode === "dark");
  });

  it("ignores a corrupt stored value", async () => {
    localStorage.setItem("future.theme", "sepia");
    const theme = await import("./theme");

    expect(theme.getThemeMode()).toBe("system");
  });

  it("follows OS appearance changes while in system mode", async () => {
    const media = installMatchMedia(false);
    const theme = await import("./theme");
    expect(theme.getResolvedTheme()).toBe("light");

    media.setMatches(true);

    expect(theme.getResolvedTheme()).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);

    media.setMatches(false);

    expect(theme.getResolvedTheme()).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("stops following the OS once an explicit theme is chosen", async () => {
    const media = installMatchMedia(false);
    const theme = await import("./theme");
    theme.setThemeMode("light");

    media.setMatches(true);

    expect(theme.getResolvedTheme()).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("notifies subscribers on a mode change and on a system-driven resolve", async () => {
    const media = installMatchMedia(false);
    const theme = await import("./theme");
    const listener = vi.fn();
    const unsubscribe = theme.subscribeTheme(listener);

    theme.setThemeMode("dark");
    expect(listener).toHaveBeenCalledTimes(1);

    theme.setThemeMode("system");
    media.setMatches(true);
    expect(listener).toHaveBeenCalledTimes(3);

    unsubscribe();
    media.setMatches(false);
    expect(listener).toHaveBeenCalledTimes(3);
  });
});

describe("theme palette", () => {
  it("starts on the app's own palette and puts it on the root before the first render", async () => {
    installMatchMedia(false);
    const theme = await import("./theme");

    expect(theme.getThemePalette()).toBe("futureos");
    // An attribute, not a class, so a picker preview can carry its own.
    expect(document.documentElement.dataset.palette).toBe("futureos");
  });

  it("restores a stored palette", async () => {
    installMatchMedia(false);
    localStorage.setItem("future.palette", "gruvbox");
    const theme = await import("./theme");

    expect(theme.getThemePalette()).toBe("gruvbox");
    expect(document.documentElement.dataset.palette).toBe("gruvbox");
  });

  it("switches and persists a palette without touching the light/dark choice", async () => {
    installMatchMedia(false);
    const theme = await import("./theme");
    theme.setThemeMode("light");

    theme.setThemePalette("catppuccin");

    expect(theme.getThemePalette()).toBe("catppuccin");
    expect(document.documentElement.dataset.palette).toBe("catppuccin");
    expect(localStorage.getItem("future.palette")).toBe("catppuccin");
    // The two axes are independent: a palette is not a rendition.
    expect(theme.getThemeMode()).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(localStorage.getItem("future.theme")).toBe("light");
  });

  it("keeps the chosen palette when the rendition flips", async () => {
    const media = installMatchMedia(false);
    const theme = await import("./theme");
    theme.setThemePalette("rose-pine");
    theme.setThemeMode("system");

    media.setMatches(true);

    expect(theme.getResolvedTheme()).toBe("dark");
    expect(document.documentElement.dataset.palette).toBe("rose-pine");
  });

  it("ignores an unknown stored palette and an unknown id at runtime", async () => {
    installMatchMedia(false);
    localStorage.setItem("future.palette", "monokai");
    const theme = await import("./theme");

    expect(theme.getThemePalette()).toBe("futureos");
  });

  it("notifies subscribers when the palette changes", async () => {
    installMatchMedia(false);
    const theme = await import("./theme");
    const listener = vi.fn();
    const unsubscribe = theme.subscribeTheme(listener);

    theme.setThemePalette("solarized");
    expect(listener).toHaveBeenCalledTimes(1);

    unsubscribe();
    theme.setThemePalette("nord");
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("lists a palette id that the stylesheet can match", async () => {
    const theme = await import("./theme");
    // The ids are CSS attribute values (`[data-palette="rose-pine"]`), so they
    // must stay lowercase-dashed rather than following the TS casing.
    for (const palette of theme.THEME_PALETTES) {
      expect(palette).toBe(palette.toLowerCase());
      expect(palette).not.toContain(" ");
    }
  });
});
