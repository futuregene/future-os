/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      // Colors live in `src/styles/globals.css` (`@theme` for the light palette
      // plus a `.dark` override block) so the whole palette can flip at runtime
      // through CSS variables. Only values that need a variable reference stay
      // here, which is why shadows point at `--shadow-*`.
      boxShadow: {
        panel: "var(--shadow-panel)",
        dialog: "var(--shadow-dialog)",
        "sidebar-divider": "var(--shadow-sidebar-divider)",
        "sidebar-floating": "var(--shadow-sidebar-floating)",
      },
    },
  },
  plugins: [],
};
