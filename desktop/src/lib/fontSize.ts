import { useSyncExternalStore } from "react";

/**
 * The reader's text size.
 *
 * Only type scales: `styles/globals.css` turns the multiplier into the
 * `--text-*` / `leading-*` variables that Tailwind's type utilities read, while
 * spacing, borders and panel widths keep their design-time pixels. The shell
 * therefore keeps its proportions and only the words grow.
 *
 * The vocabulary (and the multipliers) match the mobile app's appearance
 * setting, and `medium` is exactly 1 so the default rendering is unchanged to
 * the pixel. Like the theme, the choice is persisted in `localStorage` and read
 * before the first React paint.
 */

export const FONT_SIZES = ["small", "medium", "large", "xlarge"] as const;
export type FontSizePreference = (typeof FONT_SIZES)[number];

export const FONT_SIZE_SCALE: Record<FontSizePreference, number> = {
  small: 0.85,
  medium: 1,
  large: 1.15,
  xlarge: 1.35,
};

export const DEFAULT_FONT_SIZE: FontSizePreference = "medium";

const STORAGE_KEY = "future.fontSize";
const SCALE_VARIABLE = "--app-font-scale";

function isFontSize(value: string | null): value is FontSizePreference {
  return (FONT_SIZES as readonly string[]).includes(value ?? "");
}

function readStoredFontSize(): FontSizePreference {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (isFontSize(stored))
      return stored;
  }
  catch {
    // localStorage may be unavailable; fall through to the default.
  }
  return DEFAULT_FONT_SIZE;
}

function applyFontScale(size: FontSizePreference): void {
  if (typeof document === "undefined")
    return;
  document.documentElement.style.setProperty(SCALE_VARIABLE, String(FONT_SIZE_SCALE[size]));
}

let size: FontSizePreference = readStoredFontSize();
const listeners = new Set<() => void>();

function emitChange(): void {
  for (const listener of listeners) {
    listener();
  }
}

export function getFontSize(): FontSizePreference {
  return size;
}

export function setFontSize(next: FontSizePreference): void {
  size = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  }
  catch {
    // Persistence is best-effort; the change still applies for this session.
  }
  applyFontScale(next);
  emitChange();
}

export function subscribeFontSize(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The text size the reader picked. */
export function useFontSize(): FontSizePreference {
  return useSyncExternalStore(subscribeFontSize, getFontSize, () => DEFAULT_FONT_SIZE);
}

// Apply before the first React paint.
applyFontScale(size);
