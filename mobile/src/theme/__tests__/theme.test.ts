import {
  FONT_SIZE_SCALE,
  darkCodeColors,
  darkColors,
  lightCodeColors,
  lightColors,
  type FontSizePreference,
} from "../tokens";
import { buildTheme, darkTheme, lightTheme, resolveScheme } from "../theme";

/** WCAG relative luminance of a `#rrggbb` colour. */
function luminance(color: string): number {
  const value = Number.parseInt(color.slice(1), 16);
  const channels = [(value >> 16) & 255, (value >> 8) & 255, value & 255].map(channel => {
    const scaled = channel / 255;
    return scaled <= 0.03928 ? scaled / 12.92 : ((scaled + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0]! + 0.7152 * channels[1]! + 0.0722 * channels[2]!;
}

function contrast(foreground: string, background: string): number {
  const [higher, lower] = [luminance(foreground), luminance(background)].sort((left, right) => right - left);
  return (higher! + 0.05) / (lower! + 0.05);
}

describe("the scheme to paint", () => {
  it("follows the phone in system mode, treating an unreported scheme as light", () => {
    expect(resolveScheme("system", "dark")).toBe("dark");
    expect(resolveScheme("system", "light")).toBe("light");
    expect(resolveScheme("system", null)).toBe("light");
    expect(resolveScheme("system", undefined)).toBe("light");
  });

  it("lets a pinned mode override whatever the phone reports", () => {
    expect(resolveScheme("dark", "light")).toBe("dark");
    expect(resolveScheme("light", "dark")).toBe("light");
    expect(resolveScheme("dark", null)).toBe("dark");
    expect(resolveScheme("light", undefined)).toBe("light");
  });
});

describe("building a theme", () => {
  it("pairs each scheme with its own palette, for the app and for code", () => {
    expect(buildTheme("light", "medium").colors).toBe(lightColors);
    expect(buildTheme("light", "medium").codeColors).toBe(lightCodeColors);
    expect(buildTheme("dark", "medium").colors).toBe(darkColors);
    expect(buildTheme("dark", "medium").codeColors).toBe(darkCodeColors);
    expect(lightTheme.scheme).toBe("light");
    expect(darkTheme.scheme).toBe("dark");
    // Both palettes define the same roles, or a style would read `undefined` in
    // one scheme only — and it would do it on a screen nobody tested.
    expect(Object.keys(darkColors).sort()).toEqual(Object.keys(lightColors).sort());
    expect(Object.keys(darkCodeColors).sort()).toEqual(Object.keys(lightCodeColors).sort());
  });

  it("leaves every designed size untouched at the default text size", () => {
    // The app was drawn at `medium`; this is the promise that adding the
    // setting did not move a pixel for a reader who never opens it.
    for (const size of [10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 24, 28]) {
      expect(lightTheme.font(size)).toBe(size);
      expect(lightTheme.leading(size)).toBe(size);
    }
    expect(lightTheme.chat).toEqual({ fontSize: 15, lineHeight: 22 });
  });

  it("scales both the sizes and the message body by the reader's choice", () => {
    for (const preference of ["small", "medium", "large", "xlarge"] as FontSizePreference[]) {
      const theme = buildTheme("light", preference);
      const scale = FONT_SIZE_SCALE[preference];
      for (const size of [12, 13, 15, 22]) {
        expect(theme.font(size)).toBe(Math.round(size * scale));
        expect(theme.leading(size)).toBe(Math.round(size * scale));
      }
      expect(theme.chat.fontSize).toBe(Math.round(15 * scale));
      expect(theme.chat.lineHeight).toBe(Math.round(22 * scale));
    }
    // Whole points, and four choices the reader can actually tell apart.
    expect(buildTheme("light", "small").font(15)).toBe(13);
    expect(buildTheme("light", "large").font(15)).toBe(17);
    expect(buildTheme("light", "xlarge").font(13)).toBe(18);
    const body = (["small", "medium", "large", "xlarge"] as FontSizePreference[]).map(preference => buildTheme("light", preference).font(15));
    expect(body).toEqual([...body].sort((left, right) => left - right));
    expect(new Set(body).size).toBe(4);
  });
});

describe("the dark palette", () => {
  // Excluding the indicator colours (`warningDot`, `generating`): those are
  // shapes, and they are allowed to be brighter than text.
  const textRoles = ["ink", "inkSoft", "inkMuted", "inkStrong", "accent", "info", "success", "warning", "danger"] as const;

  it("keeps its surfaces dark and its text light", () => {
    for (const role of ["canvas", "surface", "surfaceSubtle"] as const) {
      expect(luminance(darkColors[role])).toBeLessThan(luminance(lightColors[role]));
    }
    for (const role of ["ink", "inkSoft", "inkMuted", "inkStrong"] as const) {
      expect(luminance(darkColors[role])).toBeGreaterThan(luminance(lightColors[role]));
    }
    expect(luminance(darkColors.canvas)).toBeLessThan(luminance(darkColors.surface));
    expect(luminance(darkColors.surface)).toBeLessThan(luminance(darkColors.surfaceSubtle));
    expect(luminance(darkColors.inkMuted)).toBeLessThan(luminance(darkColors.ink));
  });

  it("stays readable: every text role clears 4.5:1 on the surfaces it is drawn on", () => {
    for (const role of textRoles) {
      expect(contrast(darkColors[role], darkColors.surface)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(darkColors[role], darkColors.canvas)).toBeGreaterThanOrEqual(4.5);
    }
    // `surface` is the ink for text drawn *on* the accent: a user bubble, a
    // primary button, the send arrow. That pair has to contrast too.
    expect(contrast(darkColors.surface, darkColors.accent)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(darkColors.surface, darkColors.accentHover)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(darkColors.ink, darkColors.accentSoft)).toBeGreaterThanOrEqual(4.5);
  });

  it("keeps every code token readable on the code surface", () => {
    for (const color of Object.values(darkCodeColors)) {
      expect(contrast(color, darkColors.surfaceSubtle)).toBeGreaterThanOrEqual(4.5);
    }
  });
});
