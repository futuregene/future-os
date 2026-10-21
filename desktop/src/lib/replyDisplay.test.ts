// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * The reading setting is on unless the reader turns it off.
 *
 * The default is what a fresh install (and an install that predates the
 * setting) reads, so it is decided by the *absence* of a stored value — a test
 * that only flips the flag would never notice the default going back to off.
 */
const KEY = "future.reply.finalAnswerOnly";

afterEach(() => {
  localStorage.removeItem(KEY);
  vi.resetModules();
});

async function fresh() {
  vi.resetModules();
  return import("./replyDisplay");
}

describe("the reading setting's default", () => {
  it("is on when nothing is stored", async () => {
    const mod = await fresh();
    expect(mod.DEFAULT_FINAL_ANSWER_ONLY).toBe(true);
    expect(mod.getFinalAnswerOnly()).toBe(true);
  });

  it("is off only when the reader stored that", async () => {
    localStorage.setItem(KEY, "false");
    expect((await fresh()).getFinalAnswerOnly()).toBe(false);

    localStorage.setItem(KEY, "true");
    expect((await fresh()).getFinalAnswerOnly()).toBe(true);
  });

  it("never reads a corrupt value as an opt-in", async () => {
    // Only the one value this app writes turns the fold off; anything else —
    // a truncated write, another build's key — stays on the default.
    localStorage.setItem(KEY, "");
    expect((await fresh()).getFinalAnswerOnly()).toBe(true);
    localStorage.setItem(KEY, "0");
    expect((await fresh()).getFinalAnswerOnly()).toBe(true);
  });

  it("persists the choice the reader made", async () => {
    const mod = await fresh();
    mod.setFinalAnswerOnly(false);
    expect(localStorage.getItem(KEY)).toBe("false");
    expect(mod.getFinalAnswerOnly()).toBe(false);
    expect((await fresh()).getFinalAnswerOnly()).toBe(false);
  });
});
