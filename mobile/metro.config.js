const path = require("node:path");
const { getDefaultConfig } = require("expo/metro-config");

const config = getDefaultConfig(__dirname);

// The NATS key libraries contain guarded Node fallbacks. Metro still resolves
// those requires even though React Native supplies these globals at runtime.
config.resolver.extraNodeModules = {
  ...config.resolver.extraNodeModules,
  crypto: path.resolve(__dirname, "src/polyfills/crypto.ts"),
  util: path.resolve(__dirname, "src/polyfills/util.ts"),
  "sodium-universal": require.resolve("sodium-javascript"),
  "sodium-native": require.resolve("sodium-javascript"),
};

// MathJax uses a package-import alias for its default font. Metro does not
// resolve that external "imports" target yet. Use the same self-contained TeX
// SVG font as our renderer rather than bundling the dynamic browser font.
config.resolver.resolveRequest = (context, moduleName, platform) => context.resolveRequest(
  context,
  moduleName === "#default-font/svg/default.js"
    ? "@mathjax/mathjax-tex-font/mjs/svg/default.js"
    : moduleName,
  platform,
);

module.exports = config;
