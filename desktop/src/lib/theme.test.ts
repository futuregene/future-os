// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

import { invokeCommand } from "../integrations/tauri/invoke";

vi.mock("../integrations/tauri/invoke", () => ({
  invokeCommand: vi.fn(() => Promise.resolve()),
}));

/**
 * The native window theme is an *input* to the next resolution, not only an
 * output: a pinned `setTheme("dark")` makes the webview report a dark
 * `prefers-color-scheme`, so the mode switch back to "system" has to be
 * observed here — that pin is what kept the app dark on a light desktop.
 */
const setTheme = vi.fn<(theme: "light" | "dark" | null) => Promise<void>>(() => Promise.resolve());
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme }),
}));

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
  document.documentElement.style.removeProperty("--color-surface");
  document.documentElement.style.removeProperty("--color-ink");
  delete document.documentElement.dataset.palette;
  vi.mocked(invokeCommand).mockClear();
  setTheme.mockClear();
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

  /// The native window theme is what the webview reports back through
  /// `prefers-color-scheme`. Pinning it on "system" made "follow the system"
  /// resolve to the last explicit choice — on a light desktop, Dark → System
  /// stayed dark. Switching back must therefore *clear* the pin.
  it("clears the pinned native theme when following the system again", async () => {
    installMatchMedia(false);
    const theme = await import("./theme");

    theme.setThemeMode("dark");
    expect(setTheme).toHaveBeenLastCalledWith("dark");

    theme.setThemeMode("system");
    expect(setTheme).toHaveBeenLastCalledWith(null);
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

describe("the theme when the platform cannot answer", () => {
  it("falls back to light, and binds no listener, when matchMedia throws", async () => {
    // A webview whose media-query support is broken, or a non-DOM runner: the app
    // still has to paint, so an unanswerable preference resolves to light instead of
    // taking the first render down with it.
    vi.stubGlobal("matchMedia", () => {
      throw new Error("matchMedia is not available");
    });
    const theme = await import("./theme");

    expect(theme.getThemeMode()).toBe("system");
    expect(theme.getResolvedTheme()).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    // Switching again re-checks the listener; it must stay unbound rather than throw.
    expect(() => theme.setThemeMode("dark")).not.toThrow();
    expect(() => theme.setThemeMode("system")).not.toThrow();
    expect(theme.getResolvedTheme()).toBe("light");
  });

  it("paints nothing on the window frame when the stylesheet cannot be read", async () => {
    // No layout engine means no computed tokens; the caption colours are cosmetic, so
    // the palette change must still land.
    installMatchMedia(false);
    const computed = vi.spyOn(window, "getComputedStyle").mockImplementation(() => {
      throw new Error("no layout engine");
    });
    const theme = await import("./theme");
    vi.mocked(invokeCommand).mockClear();

    expect(() => theme.setThemePalette("gruvbox")).not.toThrow();
    expect(theme.getThemePalette()).toBe("gruvbox");
    expect(invokeCommand).not.toHaveBeenCalled();
    computed.mockRestore();
  });

  it("ignores a system notification that resolves to the theme already showing", async () => {
    // The OS re-announces its preference without changing it; repainting and
    // re-emitting would churn every subscriber for nothing.
    const media = installMatchMedia(false);
    const theme = await import("./theme");
    const listener = vi.fn();
    theme.subscribeTheme(listener);

    media.setMatches(false);

    expect(listener).not.toHaveBeenCalled();
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });
});

describe("re-reading the system preference after a pin is cleared", () => {
  it("stops after a few attempts instead of polling for ever", async () => {
    // `setTheme(null)` does not change what the webview answers within the same tick,
    // so the re-read is retried — as long as nothing changes, a bounded number of
    // times and then not again.
    installMatchMedia(false);
    const theme = await import("./theme");
    vi.useFakeTimers();
    try {
      theme.setThemeMode("system");
      expect(theme.getResolvedTheme()).toBe("light");
      // The 0 ms re-check, then the 120 ms retries. Once the fourth read has said the
      // same thing, nothing is left scheduled — which is what "stops" means here.
      await vi.advanceTimersByTimeAsync(1000);
      expect(vi.getTimerCount()).toBe(0);
      await vi.advanceTimersByTimeAsync(5000);
      expect(vi.getTimerCount()).toBe(0);
      expect(theme.getResolvedTheme()).toBe("light");
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("lands on the OS's own answer when the retry sees it", async () => {
    const media = installMatchMedia(false);
    const theme = await import("./theme");
    vi.useFakeTimers();
    try {
      theme.setThemeMode("system");
      // The native side applies the change asynchronously, so the second read is the
      // first one that sees the OS preference.
      media.setMatches(true);
      await vi.advanceTimersByTimeAsync(200);
      expect(theme.getResolvedTheme()).toBe("dark");
      expect(document.documentElement.classList.contains("dark")).toBe(true);
    }
    finally {
      vi.useRealTimers();
    }
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

  it("hands the rendered surface and ink to the window frame after a palette change", async () => {
    installMatchMedia(false);
    vi.mocked(invokeCommand).mockClear();
    document.documentElement.style.setProperty("--color-surface", "#282828");
    document.documentElement.style.setProperty("--color-ink", "#ebdbb2");
    const theme = await import("./theme");

    theme.setThemePalette("gruvbox");

    // The backend cannot map a palette id to a color; the computed tokens are
    // the only place the winning palette is known.
    expect(invokeCommand).toHaveBeenCalledWith("set_window_caption_colors", {
      caption: "#282828",
      text: "#ebdbb2",
    });
  });
});
