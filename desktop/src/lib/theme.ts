import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSyncExternalStore } from "react";

/**
 * App appearance: two independent axes.
 *
 * **Mode** (`light` / `dark` / `system`) picks which rendition is painted.
 * **Palette** picks the hues. The values live in `styles/globals.css`: `@theme`
 * holds the light values, a `:root.dark` block overrides them, and each palette
 * adds a `[data-palette="…"]` block (plus a dark override) that reassigns the
 * same variables. Switching therefore only has to toggle the `.dark` class and
 * the `data-palette` attribute on <html> — components keep using semantic
 * tokens and never branch on either axis.
 *
 * Both choices are persisted like the UI language (`localStorage`, read before
 * the first render) rather than in backend app settings: the class and the
 * attribute must be on <html> before React paints, or the window flashes the
 * wrong background.
 */

export const THEME_MODES = ["light", "dark", "system"] as const;
export type ThemeMode = (typeof THEME_MODES)[number];
export type ResolvedTheme = "light" | "dark";

/** First-run default: follow the OS, like most desktop apps. */
export const DEFAULT_THEME_MODE: ThemeMode = "system";

/**
 * The palettes, in picker order. Ids are the `data-palette` values in
 * `styles/globals.css`; labels are the user-facing names, which are proper
 * nouns and therefore live in the locale bundles only for their one-line
 * description.
 */
export const THEME_PALETTES = [
  "futureos",
  "nord",
  "solarized",
  "gruvbox",
  "rose-pine",
  "catppuccin",
] as const;
export type ThemePalette = (typeof THEME_PALETTES)[number];

/** First-run default: the app's own palette. */
export const DEFAULT_THEME_PALETTE: ThemePalette = "futureos";

const STORAGE_KEY = "future.theme";
const PALETTE_STORAGE_KEY = "future.palette";
const SYSTEM_QUERY = "(prefers-color-scheme: dark)";

function isThemeMode(value: string | null): value is ThemeMode {
  return (THEME_MODES as readonly string[]).includes(value ?? "");
}

function isThemePalette(value: string | null): value is ThemePalette {
  return (THEME_PALETTES as readonly string[]).includes(value ?? "");
}

function readStoredThemeMode(): ThemeMode {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (isThemeMode(stored))
      return stored;
  }
  catch {
    // localStorage may be unavailable; fall through to the default.
  }
  return DEFAULT_THEME_MODE;
}

function readStoredThemePalette(): ThemePalette {
  try {
    const stored = localStorage.getItem(PALETTE_STORAGE_KEY);
    if (isThemePalette(stored))
      return stored;
  }
  catch {
    // localStorage may be unavailable; fall through to the default.
  }
  return DEFAULT_THEME_PALETTE;
}

function systemPrefersDark(): boolean {
  try {
    return typeof window !== "undefined"
      && typeof window.matchMedia === "function"
      && window.matchMedia(SYSTEM_QUERY).matches;
  }
  catch {
    return false;
  }
}

function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === "system")
    return systemPrefersDark() ? "dark" : "light";
  return mode;
}

function applyResolvedTheme(theme: ResolvedTheme): void {
  if (typeof document !== "undefined") {
    const root = document.documentElement;
    root.classList.toggle("dark", theme === "dark");
    // Native controls (select popups, caret, scrollbars) follow this too.
    root.style.colorScheme = theme;
  }
  // Match the native window chrome (Windows title bar, macOS window backdrop).
  // Best-effort: outside a Tauri webview (browser preview, tests) it throws.
  try {
    void getCurrentWindow().setTheme(theme).catch(() => {});
  }
  catch {
    // Not running inside Tauri.
  }
}

/**
 * The palette is an attribute rather than a class so a Settings preview card
 * can carry its own `data-palette` and paint itself while another palette is
 * active (the CSS blocks match a nested preview as well as the root).
 */
function applyPalette(palette: ThemePalette): void {
  if (typeof document !== "undefined") {
    document.documentElement.dataset.palette = palette;
  }
}

let mode: ThemeMode = readStoredThemeMode();
let resolved: ResolvedTheme = resolveTheme(mode);
let palette: ThemePalette = readStoredThemePalette();
let systemQuery: MediaQueryList | null = null;
const listeners = new Set<() => void>();

function emitChange(): void {
  for (const listener of listeners) {
    listener();
  }
}

function handleSystemChange(): void {
  if (mode !== "system")
    return;
  const next = resolveTheme("system");
  if (next === resolved)
    return;
  resolved = next;
  applyResolvedTheme(resolved);
  emitChange();
}

/**
 * Binds the OS appearance listener lazily: non-DOM test/webview environments
 * have no `matchMedia`, and a test can install one before switching to
 * "system".
 */
function ensureSystemListener(): void {
  if (systemQuery)
    return;
  try {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function")
      return;
    systemQuery = window.matchMedia(SYSTEM_QUERY);
    systemQuery.addEventListener("change", handleSystemChange);
  }
  catch {
    systemQuery = null;
  }
}

export function getThemeMode(): ThemeMode {
  return mode;
}

export function getResolvedTheme(): ResolvedTheme {
  return resolved;
}

export function getThemePalette(): ThemePalette {
  return palette;
}

export function setThemeMode(next: ThemeMode): void {
  ensureSystemListener();
  mode = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  }
  catch {
    // Persistence is best-effort; the change still applies for this session.
  }
  const nextResolved = resolveTheme(next);
  if (nextResolved !== resolved) {
    resolved = nextResolved;
    applyResolvedTheme(resolved);
  }
  emitChange();
}

export function setThemePalette(next: ThemePalette): void {
  palette = next;
  try {
    localStorage.setItem(PALETTE_STORAGE_KEY, next);
  }
  catch {
    // Persistence is best-effort; the change still applies for this session.
  }
  applyPalette(palette);
  emitChange();
}

export function subscribeTheme(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The user's choice (`light` / `dark` / `system`). */
export function useThemeMode(): ThemeMode {
  return useSyncExternalStore(subscribeTheme, getThemeMode, () => DEFAULT_THEME_MODE);
}

/** The theme actually rendered, with "system" resolved against the OS. */
export function useResolvedTheme(): ResolvedTheme {
  return useSyncExternalStore(subscribeTheme, getResolvedTheme, () => "light");
}

/** The palette the user chose (`futureos` by default). */
export function useThemePalette(): ThemePalette {
  return useSyncExternalStore(subscribeTheme, getThemePalette, () => DEFAULT_THEME_PALETTE);
}

// Apply before the first React paint.
ensureSystemListener();
applyResolvedTheme(resolved);
applyPalette(palette);
