const { defineConfig } = require("eslint/config");
const { fixupConfigRules } = require("@eslint/compat");
const expoConfig = require("eslint-config-expo/flat");

module.exports = defineConfig([
  ...fixupConfigRules(expoConfig),
  {
    ignores: ["android/**", "ios/**", "coverage/**", "src/version.generated.ts"],
    settings: {
      // Resolve from Expo's config, not from dependencies traversed by import/namespace.
      "import/resolver": require.resolve("eslint-import-resolver-typescript", {
        paths: [require.resolve("eslint-config-expo/flat")],
      }),
    },
    rules: {
      eqeqeq: "error",
      "no-console": ["error", { allow: ["warn", "error"] }],
      "react-hooks/exhaustive-deps": "error",
    },
  },
  {
    // The namespace rule recursively loads the linked ESM-only Remark graph
    // through eslint-plugin-import's legacy resolver and rejects its interface.
    // TypeScript still validates every named import during `typecheck`.
    files: ["src/components/MarkdownText.tsx", "src/remote/localPath.ts"],
    rules: { "import/namespace": "off" },
  },
]);
