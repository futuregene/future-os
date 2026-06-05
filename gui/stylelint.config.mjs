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
};
