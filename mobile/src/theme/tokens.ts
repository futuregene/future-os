/**
 * Raw design tokens: the two palettes and the metrics that do not change with
 * the theme or the reader's text size.
 *
 * Colours are never read from here at a call site. A component asks a theme
 * (`src/theme/ThemeContext.tsx`) for its palette, so a screen is drawn once in
 * the scheme and text size the reader chose; the style objects that consume
 * them are built per theme by `createThemedStyles`.
 */

export type ThemeScheme = "light" | "dark";

/** Every colour the app paints with, in both schemes. */
export type Palette = {
  /** Page background, behind cards and lists. */
  canvas: string;
  /** Content surface: bubbles, cards, sheets, the chat background. */
  surface: string;
  /** A recessed surface inside `surface`: code blocks, pressed rows, table heads. */
  surfaceSubtle: string;
  line: string;
  lineSoft: string;
  /** Primary text. */
  ink: string;
  /** Secondary text: labels, tool details. */
  inkSoft: string;
  /** Tertiary text: hints, timestamps, placeholders. */
  inkMuted: string;
  /** Headings and other text that must outrank `ink`. */
  inkStrong: string;
  /** Brand: links, selection, icons, and the fill behind `surface`-coloured text. */
  accent: string;
  /** Accent-tinted background for the selected state. */
  accentSoft: string;
  accentHover: string;
  accentDisabled: string;
  focus: string;
  success: string;
  successSoft: string;
  successLine: string;
  danger: string;
  dangerSoft: string;
  dangerLine: string;
  warning: string;
  warningSoft: string;
  warningLine: string;
  /**
   * The yellow an indicator dot wears. A dot is a shape, not text, so it wants
   * the hue itself rather than `warning`'s darkened-for-readability amber: at
   * 8px that one is orange enough to sit next to the `danger` red beside it,
   * while this one is unmistakably the middle of green / yellow / red.
   */
  warningDot: string;
  info: string;
  infoSoft: string;
  infoLine: string;
  /** The pulsing dot of a run that is still producing output. */
  generating: string;
  /** Scrim behind modals and sheets. */
  overlay: string;
};

export const lightColors: Palette = {
  canvas: "#f5f7fb",
  surface: "#ffffff",
  surfaceSubtle: "#f1f4f8",
  line: "#d9dee7",
  lineSoft: "#e8edf4",
  ink: "#172033",
  inkSoft: "#5d687a",
  // Tertiary text is small (12–13pt) wherever it is used, so it is held to the
  // 4.5:1 floor of WCAG AA against every surface it is painted on — the darkest
  // of them is `accentSoft`, where this grey still reads at 4.6:1. The previous
  // #707c90 sat at 3.7–4.2:1 on the same surfaces: legible on white, thin on a
  // tinted row for timestamps, hints and section captions.
  inkMuted: "#606c82",
  inkStrong: "#0f172a",
  accent: "#2563eb",
  accentSoft: "#e8f0ff",
  accentHover: "#1d4ed8",
  accentDisabled: "#bfdbfe",
  focus: "#93c5fd",
  success: "#15803d",
  successSoft: "#f0fdf4",
  successLine: "#bbf7d0",
  danger: "#dc2626",
  dangerSoft: "#fef2f2",
  dangerLine: "#fecaca",
  warning: "#b45309",
  warningSoft: "#fffbeb",
  warningLine: "#fde68a",
  warningDot: "#facc15",
  info: "#1d4ed8",
  infoSoft: "#eff6ff",
  infoLine: "#bfdbfe",
  generating: "#f59e0b",
  overlay: "rgba(0, 0, 0, 0.6)",
};

/**
 * The same roles at night.
 *
 * Two rules shape it. Text tints carry the hue instead of a darkened version of
 * it (`accent` is a lighter blue, not a darker one), because a colour chosen to
 * be legible *on* white is far too dark *on* a dark surface. And the roles that
 * paint a background keep a light value, because everything drawn on top of
 * them — `surface`-coloured label text on an accent bubble or button — is
 * itself the dark ink of this scheme; the pair is what has to contrast.
 */
export const darkColors: Palette = {
  canvas: "#0b1220",
  surface: "#151e2e",
  surfaceSubtle: "#212c40",
  line: "#33415c",
  lineSoft: "#26314a",
  ink: "#e6ecf6",
  inkSoft: "#a9b6cb",
  inkMuted: "#8894ab",
  inkStrong: "#f4f8ff",
  accent: "#60a5fa",
  accentSoft: "#1e2c4b",
  accentHover: "#93c5fd",
  accentDisabled: "#33456e",
  focus: "#3b82f6",
  success: "#4ade80",
  successSoft: "#12261b",
  successLine: "#1f5136",
  danger: "#f87171",
  dangerSoft: "#2b1719",
  dangerLine: "#5e2b2e",
  warning: "#fbbf24",
  warningSoft: "#2b2312",
  warningLine: "#5d4a1c",
  warningDot: "#facc15",
  info: "#7aa8fb",
  infoSoft: "#16223a",
  infoLine: "#33507f",
  generating: "#fbbf24",
  overlay: "rgba(0, 0, 0, 0.7)",
};

/** Syntax categories on the code surface (not status colours). */
export type CodePalette = {
  comment: string;
  keyword: string;
  literal: string;
  string: string;
  function: string;
  property: string;
  operator: string;
  /** JSON preview roles, which split the source categories differently. */
  jsonKey: string;
  jsonString: string;
  jsonNumber: string;
  jsonLiteral: string;
};

export const lightCodeColors: CodePalette = {
  comment: "#596579",
  keyword: "#a62646",
  literal: "#075eab",
  string: "#17633c",
  function: "#6f42a6",
  property: "#805000",
  operator: "#4c566a",
  jsonKey: "#1d4ed8",
  jsonString: "#047857",
  jsonNumber: "#b45309",
  jsonLiteral: "#7c3aed",
};

export const darkCodeColors: CodePalette = {
  comment: "#8b98b0",
  keyword: "#f38ba8",
  literal: "#7dd3fc",
  string: "#86efac",
  function: "#c4b5fd",
  property: "#fcd34d",
  operator: "#cbd5e1",
  jsonKey: "#7aa8fb",
  jsonString: "#86efac",
  jsonNumber: "#fbbf24",
  jsonLiteral: "#c4b5fd",
};

/**
 * The reader's text size.
 *
 * `medium` is the scale every style in the app was designed at, and it maps to
 * exactly 1 so the default rendering is unchanged to the pixel.
 */
export type FontSizePreference = "small" | "medium" | "large" | "xlarge";

export const FONT_SIZE_SCALE: Record<FontSizePreference, number> = {
  small: 0.85,
  medium: 1,
  large: 1.15,
  xlarge: 1.35,
};

export const FONT_SIZE_PREFERENCE_ORDER: readonly FontSizePreference[] = ["small", "medium", "large", "xlarge"];

export const spacing = {
  xs: 4,
  sm: 8,
  md: 12,
  lg: 16,
  xl: 24,
  xxl: 32,
} as const;

// Softer mobile surfaces: small chips, controls, cards, and modal sheets.
// `pill` stays fully round for capsule chips and status dots.
export const radius = {
  sm: 6,
  md: 12,
  lg: 16,
  xl: 24,
  pill: 999,
} as const;

// Keep screen gutters and minimum touch targets consistent across phone sizes.
export const layout = {
  gutter: 16,
  touchTarget: 44,
  contentMaxWidth: 760,
  formMaxWidth: 560,
  dialogMaxWidth: 420,
} as const;
