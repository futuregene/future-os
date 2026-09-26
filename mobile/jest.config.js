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
  // Cap the pool below the machine's core count. The default (`cpus - 1`)
  // assumes the whole box belongs to this suite; on a shared machine (a
  // developer's laptop also running a Rust build, or CI running several jobs)
  // it oversubscribes and CPU-starved tests blow through Jest's 5 s budget with
  // wall-clock work they finish in a fraction of that when they get a core.
  // Halving the pool removes the starvation instead of raising the timeout.
  maxWorkers: "50%",
  // Unified/Remark and their syntax-tree utilities are ESM-only. Metro handles
  // them directly; Jest needs Babel to transform the dependency chain. Keep the
  // preset's two explicit plugin exclusions, but do not skip package sources.
  transformIgnorePatterns: [
    "/node_modules/react-native-reanimated/plugin/",
    "/node_modules/@react-native/babel-preset/",
  ],
  // Measure every source file, not a hand-picked subtree: the goal is 100% of
  // coverable lines for the whole module, and a narrow list hides the gaps.
  // Test files, type declarations and the version file generated at build time
  // are not coverable source.
  collectCoverageFrom: [
    "src/**/*.{ts,tsx}",
    "!src/**/__tests__/**",
    "!src/**/*.test.{ts,tsx}",
    "!src/**/*.d.ts",
    "!src/version.generated.ts",
  ],
  // json-summary is the machine-readable report the coverage verifier reads:
  // mobile/coverage/coverage-summary.json.
  coverageReporters: ["text", "json", "json-summary", "lcov"],
  coveragePathIgnorePatterns: ["/node_modules/"],
};
