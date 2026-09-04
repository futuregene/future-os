const path = require("node:path");

const reactDirectory = path.dirname(require.resolve("react/package.json"));

module.exports = {
  preset: "jest-expo",
  // Hoisted renderers and shared packages must use Mobile's Expo-compatible
  // React instance, including JSX runtimes, rather than the desktop copy.
  moduleNameMapper: {
    "^react$": require.resolve("react"),
    "^react/(.*)$": `${reactDirectory}/$1`,
  },
  testMatch: ["<rootDir>/src/**/__tests__/**/*.test.ts"],
  // Unified/Remark and their syntax-tree utilities are ESM-only. Metro handles
  // them directly; Jest needs Babel to transform the dependency chain. Keep the
  // preset's two explicit plugin exclusions, but do not skip package sources.
  transformIgnorePatterns: [
    "/node_modules/react-native-reanimated/plugin/",
    "/node_modules/@react-native/babel-preset/",
  ],
  collectCoverageFrom: ["src/remote/**/*.ts", "!src/remote/client.ts"],
};
