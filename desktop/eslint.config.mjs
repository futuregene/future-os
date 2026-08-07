import antfu from "@antfu/eslint-config";
import tailwindcss from "eslint-plugin-tailwindcss";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = dirname(fileURLToPath(import.meta.url));
// eslint-plugin-tailwindcss v4 reads the project's main CSS entry (the file that
// `@import "tailwindcss"`), not tailwind.config.js — see the v3→v4 upgrade guide.
const cssConfigPath = join(rootDir, "src/styles/globals.css");

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
  tailwindcss.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    // v4 moved per-rule options into shared settings: `callees` → `functions`
    // (class strings are composed via `cn`), `config` → `cssConfigPath`.
    settings: {
      tailwindcss: {
        cssConfigPath,
        functions: ["cn"],
      },
    },
    rules: {
      "no-alert": "off",
      "no-console": "error",
      "react/set-state-in-effect": "off",
      // Restore the project's prior Tailwind lint baseline: ordering / shorthand /
      // custom-classname stay off (the codebase has intentional class ordering and
      // custom utilities like `floating-scrollbar`).
      "tailwindcss/classnames-order": "off",
      "tailwindcss/enforces-shorthand": "off",
      "tailwindcss/no-custom-classname": "off",
      // Downgraded error → warn: v4 flags `divide-{color}` + `border-{color}` on the
      // same element as contradicting, but they set different properties (child
      // dividers vs. the element border) and legitimately coexist.
      "tailwindcss/no-contradicting-classname": "warn",
    },
  },
);
