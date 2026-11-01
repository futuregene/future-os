import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Loading KaTeX on demand.
 *
 * The module is a ~460 kB chunk pulled in by the first formula, so the interesting
 * behaviour is not the formula but the *cache*: one load is shared by every formula on
 * screen, and a load that fails must be forgotten rather than inherited — otherwise one
 * missing chunk would leave every formula in the conversation blank for the rest of the
 * session, with no way to retry.
 */

beforeEach(() => {
  vi.resetModules();
});

/**
 * Arrange the next `import("katex")` to fail or to succeed.
 *
 * Registered per test with `vi.doMock` rather than one hoisted factory gated on a
 * mutable flag: a mock factory whose evaluation already succeeded is cached, and
 * `vi.resetModules()` does not re-run it (measured — flipping a flag afterwards
 * still resolved). A factory that *throws* is not cached, which is exactly the
 * asymmetry this file is about, so the arrangement has to happen before the
 * import rather than inside the factory.
 */
function mockKatex(available: boolean) {
  vi.doMock("katex", () => {
    if (!available)
      throw new Error("chunk unavailable");
    return { default: { renderToString: (code: string) => `<span class="katex">${code}</span>` } };
  });
}

afterEach(() => {
  vi.doUnmock("katex");
});

describe("loadKatex", () => {
  it("forgets a failed load so the next formula retries", async () => {
    mockKatex(false);
    const lazy = await import("./lazyKatex");
    const first = lazy.loadKatex();
    await expect(first).rejects.toThrow();

    // The failure is not cached: this is a new attempt, not the dead promise.
    const second = lazy.loadKatex();
    expect(second).not.toBe(first);
    await expect(second).rejects.toThrow();

    // And a chunk that does arrive is the one every later caller gets.
    mockKatex(true);
    const third = lazy.loadKatex();
    await expect(third).resolves.toHaveProperty("renderToString");
    expect(lazy.loadKatex()).toBe(third);
  });

  it("shares the one load between callers", async () => {
    mockKatex(true);
    const lazy = await import("./lazyKatex");
    const a = lazy.loadKatex();
    const b = lazy.loadKatex();
    expect(a).toBe(b);
    // One import for both, which is the point of the in-flight cache: a second caller
    // joins the promise rather than pulling the chunk again.
    await expect(a).resolves.toBeTruthy();
    expect(a).toBe(b);
  });
});

describe("mathHtml", () => {
  it("paints the formula through KaTeX", async () => {
    mockKatex(true);
    const lazy = await import("./lazyKatex");
    const katex = await lazy.loadKatex();
    expect(lazy.mathHtml(katex, "x^2", true)).toContain("x^2");
  });

  it("keeps a formula that KaTeX still refuses as its own escaped line", async () => {
    // `throwOnError: false` paints most mistakes in red; this is the residue, and it
    // must cost its own line and not the message around it.
    const { mathHtml } = await import("./lazyKatex");
    const throwing = {
      renderToString: () => {
        throw new Error("ParseError");
      },
    } as unknown as Parameters<typeof mathHtml>[0];

    const html = mathHtml(throwing, "<a&b>", false);

    expect(html).toContain("&lt;a&amp;b&gt;");
    expect(html).toContain("text-danger");
  });
});
