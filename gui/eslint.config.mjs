import antfu from "@antfu/eslint-config";
import tailwindcss from "eslint-plugin-tailwindcss";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = dirname(fileURLToPath(import.meta.url));
const tailwindConfigPath = join(rootDir, "tailwind.config.js");

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
  ...tailwindcss.configs["flat/recommended"],
  {
    files: ["src/**/*.{ts,tsx}"],
    rules: {
      "no-alert": "off",
      "no-console": "error",
      "react/set-state-in-effect": "off",
      "tailwindcss/classnames-order": "off",
      "tailwindcss/enforces-shorthand": "off",
      "tailwindcss/no-custom-classname": "off",
      "tailwindcss/no-contradicting-classname": "warn",
    },
    settings: {
      tailwindcss: {
        callees: ["cn"],
        config: tailwindConfigPath,
      },
    },
  },
);
