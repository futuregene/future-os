export default {
  extends: [
    "stylelint-config-recommended",
    "stylelint-config-tailwindcss",
  ],
  ignoreFiles: [
    "dist/**",
    "target/**",
    "node_modules/**",
    "src-tauri/**",
  ],
  rules: {
    // Tailwind resolves theme(...) during the CSS build.
    "declaration-property-value-no-unknown": [true, {
      ignoreProperties: { "/.*/": ["/^theme\\(/"] },
    }],
  },
};
