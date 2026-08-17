module.exports = {
  preset: "jest-expo",
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
