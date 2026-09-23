/**
 * Jest setup shared by every Mobile test.
 *
 * AsyncStorage is a native module, so importing it in Node throws until it is
 * replaced. Components deep in the tree now reach it — the skill-recommendation
 * budget reads the phone's daily record from it — so the replacement belongs
 * here rather than in whichever test happens to import such a component.
 *
 * The package ships the mock; this wires it up. A test that needs different
 * behaviour (e.g. an in-memory store it can inspect) still calls `jest.mock`
 * itself and wins.
 */
jest.mock("@react-native-async-storage/async-storage", () =>
  require("@react-native-async-storage/async-storage/jest/async-storage-mock"),
);
