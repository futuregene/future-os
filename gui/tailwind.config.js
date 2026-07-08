/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        // ─── Neutrals / surfaces ──────────────────────────────
        canvas: "#f6f7f9",
        surface: "#ffffff",
        "surface-subtle": "#f1f4f8",
        line: "#d9dee7",
        "line-soft": "#e8edf4",
        ink: "#172033",
        "ink-soft": "#5d687a",
        "ink-muted": "#8a94a6",
        "ink-strong": "#0f172a",
        // ─── Accent ───────────────────────────────────────────
        accent: "#2563eb",
        "accent-soft": "#e8f0ff",
        "accent-hover": "#1d4ed8",
        "accent-disabled": "#bfdbfe",
        focus: "#93c5fd",
        // ─── Status tones (text / soft bg / line) ─────────────
        success: "#15803d",
        "success-soft": "#f0fdf4",
        "success-line": "#bbf7d0",
        danger: "#dc2626",
        "danger-soft": "#fef2f2",
        "danger-line": "#fecaca",
        warning: "#b45309",
        "warning-soft": "#fffbeb",
        "warning-line": "#fde68a",
        info: "#1d4ed8",
        "info-soft": "#eff6ff",
        "info-line": "#bfdbfe",
        // ─── Activity ─────────────────────────────────────────
        // Live "generating" indicator (streaming reply) — amber ping dot.
        generating: "#f59e0b",
        // ─── Scrollbar (classic webkit thumb, see styles/globals.css) ──
        scrollbar: "#c8ced9",
        "scrollbar-hover": "#aeb7c6",
        // ─── Diff ─────────────────────────────────────────────
        "diff-add": "#e6ffec",
        "diff-add-line": "#aadfb8",
        "diff-remove": "#ffebe9",
        "diff-remove-line": "#ffc9c9"
      },
      boxShadow: {
        panel: "0 1px 2px rgba(23, 32, 51, 0.06), 0 12px 28px rgba(23, 32, 51, 0.07)",
        dialog: "0 24px 60px rgba(15, 23, 42, 0.18)",
        "sidebar-divider": "inset -8px 0 16px -16px rgba(23, 32, 51, 0.15)",
        "sidebar-floating": "6px 0 16px rgba(23, 32, 51, 0.05)"
      }
    }
  },
  plugins: []
};
