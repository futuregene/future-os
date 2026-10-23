module.exports = {
  testEnvironment: "node",
  testMatch: ["**/tests/**/*.test.ts"],
  transform: { "^.+\\.[jt]s$": [require.resolve("babel-jest"), { configFile: require.resolve("./babel.config.cjs") }] },
  moduleNameMapper: { "^sodium-universal$": require.resolve("sodium-javascript") },
  setupFiles: ["<rootDir>/tests/setup.cjs"],
  maxWorkers: 2,
  collectCoverageFrom: ["src/**/*.ts", "!src/**/*.d.ts"],
};
