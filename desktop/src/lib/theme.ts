import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSyncExternalStore } from "react";

import { invokeCommand } from "../integrations/tauri/invoke";

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
 * The palettes, in picker order: the shipped default first, then alphabetical
 * by id so the list reads as a catalogue rather than as a history of additions.
 *
 * The ids are the `data-palette` values in `styles/globals.css`, and the values
 * behind them live in `src/theme/palettes.json` — this list exists so the
 * choice is a closed union at compile time (a typo is a type error, not a
 * silently unsaved preference). `styles/globals.test.ts` and
 * `theme/palettes.test.ts` fail if it drifts from the JSON.
 *
 * Labels are proper nouns, so they live in the locale bundles (with their
 * one-line description) rather than here.
 */
export const THEME_PALETTES = [
  "futureos",
  "ayu",
  "catppuccin",
  "dracula",
  "everforest",
  "gruvbox",
  "kanagawa",
  "nightfox",
  "nord",
  "one-dark",
  "rose-pine",
  "solarized",
  "tokyo-night",
  "vesper",
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

/** The six-digit hex a palette token is written as (see `theme/palettes.json`). */
const HEX_COLOR = /^#[0-9a-f]{6}$/i;

/**
 * Hand the rendered surface/ink colors to the native window frame.
 *
 * Windows draws the title bar itself (DWM), so it does not follow the page: the
 * backend pins the caption to `--color-surface` and the title text to
 * `--color-ink` on request, and each palette overrides those variables. The
 * computed values are the only place the winning palette is known, so this
 * reads them after the class/attribute change instead of sending a light/dark
 * flag. macOS and Linux ignore the command; outside a Tauri webview the call
 * throws or rejects, which is not worth reporting.
 */
function syncWindowCaptionColors(): void {
  if (typeof document === "undefined" || typeof window === "undefined")
    return;
  let caption: string;
  let text: string;
  try {
    const styles = window.getComputedStyle(document.documentElement);
    caption = styles.getPropertyValue("--color-surface").trim();
    text = styles.getPropertyValue("--color-ink").trim();
  }
  catch {
    // No layout engine (a non-DOM test runner); nothing to paint.
    return;
  }
  if (!HEX_COLOR.test(caption) || !HEX_COLOR.test(text))
    return;
  try {
    void invokeCommand("set_window_caption_colors", { caption, text }).catch(() => {});
  }
  catch {
    // Not running inside Tauri.
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
  syncWindowCaptionColors();
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
    syncWindowCaptionColors();
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
  syncWindowCaptionColors();
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
syncWindowCaptionColors();
