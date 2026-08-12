module.exports = {
  preset: "jest-expo",
  testMatch: ["<rootDir>/src/**/__tests__/**/*.test.ts"],
  collectCoverageFrom: ["src/remote/**/*.ts", "!src/remote/client.ts"],
  // The shared thread-projection package is a `file:` symlink resolved to its
  // real path (outside node_modules), so jest-expo's node_modules allowlist
  // doesn't apply — babel-jest would transform its already-compiled CJS dist
  // and inject @babel/runtime helpers that can't resolve from there. Skip it.
  transformIgnorePatterns: ["/thread-projection/dist/"],
};
