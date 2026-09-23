const path = require("node:path");

const reactDirectory = path.dirname(require.resolve("react/package.json"));

module.exports = {
  preset: "jest-expo",
  // Hoisted renderers and shared packages must use Mobile's Expo-compatible
  // React instance, including JSX runtimes, rather than the desktop copy.
  moduleNameMapper: {
    // Exercise the same pure-JS sodium backend Metro uses, not Node native addons.
    "^sodium-universal$": require.resolve("sodium-javascript"),
    "^react$": require.resolve("react"),
    "^react/(.*)$": `${reactDirectory}/$1`,
  },
  // roots already restrict discovery to mobile. Avoid expanding <rootDir>
  // into a glob: Windows paths containing /.claude/ become mixed separators.
  testMatch: ["**/src/**/__tests__/**/*.test.ts"],
  // AsyncStorage is a native module, so importing it in Node throws until it is
  // replaced; the setup file installs the package's mock. See jest.setup.js.
  setupFiles: ["<rootDir>/jest.setup.js"],
  // Unified/Remark and their syntax-tree utilities are ESM-only. Metro handles
  // them directly; Jest needs Babel to transform the dependency chain. Keep the
  // preset's two explicit plugin exclusions, but do not skip package sources.
  transformIgnorePatterns: [
    "/node_modules/react-native-reanimated/plugin/",
    "/node_modules/@react-native/babel-preset/",
  ],
  collectCoverageFrom: ["src/remote/**/*.ts", "!src/remote/client.ts"],
};
