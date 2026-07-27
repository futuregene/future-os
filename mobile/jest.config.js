module.exports = {
  preset: "jest-expo",
  testMatch: ["<rootDir>/src/**/__tests__/**/*.test.ts"],
  collectCoverageFrom: ["src/remote/**/*.ts", "!src/remote/client.ts"],
};
