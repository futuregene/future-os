import parser from "@typescript-eslint/parser";
import plugin from "@typescript-eslint/eslint-plugin";

export default [{
  files: ["**/*.ts"],
  languageOptions: { parser, parserOptions: { sourceType: "module" } },
  plugins: { "@typescript-eslint": plugin },
  rules: {
    ...plugin.configs.recommended.rules,
    "@typescript-eslint/no-non-null-assertion": "off",
    "no-console": ["error", { allow: ["warn", "error"] }],
    eqeqeq: "error",
  },
}];
