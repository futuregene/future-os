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
