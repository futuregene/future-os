import {
  FONT_SIZE_SCALE,
  darkCodeColors,
  darkColors,
  lightCodeColors,
  lightColors,
  type CodePalette,
  type FontSizePreference,
  type Palette,
  type ThemeScheme,
} from "./tokens";
import type { ThemeMode } from "./preferences";

/** The message body scale, shared by rendered messages and the composer. */
const chatType = { fontSize: 15, lineHeight: 22 };

/**
 * Everything a styled component reads: the scheme's palette, the reader's text
 * size, and the size function that applies it.
 *
 * The type is deliberately data, not a context of its own — one object per
 * (scheme, text size) pair is what `createThemedStyles` memoises on, so both
 * settings change through the same value.
 */
export type Theme = {
  scheme: ThemeScheme;
  colors: Palette;
  codeColors: CodePalette;
  /** The message body scale at the reader's size. */
  chat: { fontSize: number; lineHeight: number };
  /** A design-time font size, at the reader's text size. */
  font(size: number): number;
  /** A design-time line height, at the reader's text size. */
  leading(size: number): number;
};

export function buildTheme(scheme: ThemeScheme, textSize: FontSizePreference): Theme {
  const scale = FONT_SIZE_SCALE[textSize];
  // Whole points, and a small-size floor — two reasons for one clamp.
  //
  // Whole points: fractional sizes are legal on both platforms but render as
  // uneven glyph weights inside a line, and the reader cannot see the
  // difference between 17.2 and 17.
  //
  // The floor: "Small" (0.85) took the 10 and 11 design sizes to 9px — the
  // pending-attachment size in the composer and the approval card's uppercase
  // labels. Short CJK strings are unreadable rather than merely quiet at that
  // size, which makes it a defect in the size setting rather than a matter of
  // taste. 10 is the same floor the desktop's `--text-3xs` / `--text-2xs` keep
  // (`desktop/src/styles/globals.css`), so the two apps answer "how small may
  // text get" the same way. It binds on "Small" alone: the smallest line height
  // the app designs with is `leading(14)`, which lands at 12 there, and scaling
  // up never approaches it.
  //
  // It stays in this one function on purpose: `leading` reads the same clamp,
  // and every line height clears the floor, so a second function would only be
  // a second place to keep in step.
  const scaleSize = (size: number) => Math.max(10, Math.round(size * scale));
  return {
    scheme,
    colors: scheme === "dark" ? darkColors : lightColors,
    codeColors: scheme === "dark" ? darkCodeColors : lightCodeColors,
    chat: { fontSize: scaleSize(chatType.fontSize), lineHeight: scaleSize(chatType.lineHeight) },
    // Font sizes and line heights take the same scale, so the leading a style
    // was designed with survives at every text size.
    font: scaleSize,
    leading: scaleSize,
  };
}

/** The theme a component gets with no provider: light, at the default size. */
export const lightTheme = buildTheme("light", "medium");
export const darkTheme = buildTheme("dark", "medium");

/**
 * The scheme to paint: the reader's mode, resolved against the phone.
 *
 * `system` takes whatever the platform reports; a phone that reports nothing
 * (`null`, or a platform without a preference) is light, which is what the app
 * has always been.
 */
export function resolveScheme(mode: ThemeMode, systemScheme: string | null | undefined): ThemeScheme {
  if (mode !== "system") return mode;
  return systemScheme === "dark" ? "dark" : "light";
}
