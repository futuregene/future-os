const path = require("node:path");
const { getDefaultConfig } = require("expo/metro-config");

const config = getDefaultConfig(__dirname);

// nats.ws and nkeys.js contain guarded Node fallbacks. Metro still resolves
// those requires even though React Native supplies these globals at runtime.
config.resolver.extraNodeModules = {
  ...config.resolver.extraNodeModules,
  crypto: path.resolve(__dirname, "src/polyfills/crypto.ts"),
  util: path.resolve(__dirname, "src/polyfills/util.ts"),
};

module.exports = config;
