export const colors = {
  canvas: "#f6f7f9",
  surface: "#ffffff",
  surfaceSubtle: "#f1f4f8",
  line: "#d9dee7",
  lineSoft: "#e8edf4",
  ink: "#172033",
  inkSoft: "#5d687a",
  inkMuted: "#8a94a6",
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
  info: "#1d4ed8",
  infoSoft: "#eff6ff",
  infoLine: "#bfdbfe",
  generating: "#f59e0b",
  overlay: "rgba(0, 0, 0, 0.6)",
} as const;

export const spacing = {
  xs: 4,
  sm: 8,
  md: 12,
  lg: 16,
  xl: 24,
  xxl: 32,
} as const;

// Corner radii mirror the desktop Tailwind scale (rounded-sm/md/lg = 4/6/8px) so
// cards, buttons and bubbles read the same on phone and desktop. `pill` stays a
// full round for capsule chips/dots.
export const radius = {
  sm: 4,
  md: 6,
  lg: 8,
  pill: 999,
} as const;
