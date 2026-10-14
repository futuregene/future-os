import { describe, expect, test } from "vitest";

import { HISTORY_PAGE_USER_EXCHANGES, HISTORY_TAIL_CURSOR } from "./historyPage";

/**
 * These two numbers are the whole reason the module exists: both remote clients
 * have to agree on them, and the host has its own copy of the second one. Pinned
 * as literals so a change has to be deliberate here rather than a quiet edit on
 * one side.
 */
describe("the history page contract", () => {
  test("both clients ask for the same number of user exchanges", () => {
    expect(HISTORY_PAGE_USER_EXCHANGES).toBe(3);
  });

  test("the tail cursor is the literal the host compares against", () => {
    // `NEWEST_PAGE_CURSOR: i64 = 9_007_199_254_740_991` in
    // `desktop/src-tauri/src/remote_host/business/history.rs`, which treats
    // anything at or above it as a first paint and anything below as a
    // scrolled-up page.
    expect(HISTORY_TAIL_CURSOR).toBe(9_007_199_254_740_991);
  });
});
