/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        canvas: "#f6f7f9",
        surface: "#ffffff",
        "surface-subtle": "#f1f4f8",
        line: "#d9dee7",
        "line-soft": "#e8edf4",
        ink: "#172033",
        "ink-soft": "#5d687a",
        "ink-muted": "#8a94a6",
        accent: "#2563eb",
        "accent-soft": "#e8f0ff"
      },
      boxShadow: {
        panel: "0 1px 2px rgba(23, 32, 51, 0.06), 0 12px 28px rgba(23, 32, 51, 0.07)"
      }
    }
  },
  plugins: []
};
