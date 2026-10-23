import { describe, expect, it } from "vitest";
import { cn } from "./cn";

/**
 * `tailwind-merge` resolves conflicts from its own table of Tailwind classes, and
 * an unknown `text-*` is a *colour* to it — so the size steps this app declares
 * itself were being dropped the moment they shared a `cn()` with an ink token.
 * Nothing threw: the label just rendered at the inherited size.
 */
describe("cn", () => {
  it("keeps a size declared in globals.css when a colour follows it", () => {
    expect(cn("text-13 font-medium text-ink-soft")).toBe("text-13 font-medium text-ink-soft");
  });

  it("still resolves a size conflict in the caller's favour", () => {
    expect(cn("text-xs", "text-13")).toBe("text-13");
  });

  it("still resolves a colour conflict in the caller's favour", () => {
    expect(cn("text-ink-soft", "text-ink")).toBe("text-ink");
  });
});
