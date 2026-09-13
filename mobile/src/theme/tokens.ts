export const colors = {
  canvas: "#f5f7fb",
  surface: "#ffffff",
  surfaceSubtle: "#f1f4f8",
  line: "#d9dee7",
  lineSoft: "#e8edf4",
  ink: "#172033",
  inkSoft: "#5d687a",
  inkMuted: "#707c90",
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

// A shared body scale keeps rendered messages and the composer in sync.
export const chatTypography = { fontSize: 15, lineHeight: 22 } as const;

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
