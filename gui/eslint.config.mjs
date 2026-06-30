import antfu from "@antfu/eslint-config";

export default antfu(
  {
    type: "app",
    react: true,
    typescript: true,
    formatters: false,
    stylistic: {
      indent: 2,
      quotes: "double",
      semi: true,
    },
    ignores: [
      "dist/**",
      "target/**",
      "node_modules/**",
      "src-tauri/**",
    ],
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    rules: {
      "no-alert": "off",
      "no-console": "error",
      "react/set-state-in-effect": "off",
    },
  },
);
