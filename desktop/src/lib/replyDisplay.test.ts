// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * The reading level is `default` unless the reader picked `detailed`.
 *
 * The level is what a fresh install (and an install that predates it) reads, so
 * it is decided by the *absence* of a stored value — a test that only sets the
 * level would never notice the default changing.
 */
const KEY = "future.reply.detail";

afterEach(() => {
  localStorage.removeItem(KEY);
  localStorage.removeItem("future.reply.finalAnswerOnly");
  vi.resetModules();
});

async function fresh() {
  vi.resetModules();
  return import("./replyDisplay");
}

describe("the reply detail level", () => {
  it("is `default` when nothing is stored", async () => {
    const mod = await fresh();
    expect(mod.DEFAULT_REPLY_DETAIL).toBe("default");
    expect(mod.getReplyDetail()).toBe("default");
    expect(mod.foldsSettledReply(mod.getReplyDetail())).toBe(true);
  });

  it("reads the stored level back, and only folds on `default`", async () => {
    localStorage.setItem(KEY, "detailed");
    const detailed = await fresh();
    expect(detailed.getReplyDetail()).toBe("detailed");
    expect(detailed.foldsSettledReply("detailed")).toBe(false);

    localStorage.setItem(KEY, "default");
    const folded = await fresh();
    expect(folded.getReplyDetail()).toBe("default");
    expect(folded.foldsSettledReply("default")).toBe(true);
  });

  it("ignores a value it never wrote", async () => {
    localStorage.setItem(KEY, "verbose");
    expect((await fresh()).getReplyDetail()).toBe("default");
  });

  it("does not read the retired boolean as a level", async () => {
    // The setting used to be a switch under this key. `false` there cannot be
    // told apart from one an older default wrote, so the level starts fresh.
    localStorage.setItem("future.reply.finalAnswerOnly", "false");
    expect((await fresh()).getReplyDetail()).toBe("default");
  });

  it("persists the level the reader picked", async () => {
    const mod = await fresh();
    mod.setReplyDetail("detailed");
    expect(localStorage.getItem(KEY)).toBe("detailed");
    expect(mod.getReplyDetail()).toBe("detailed");
    expect((await fresh()).getReplyDetail()).toBe("detailed");
  });
});
