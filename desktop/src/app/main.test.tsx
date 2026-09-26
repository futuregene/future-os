// @vitest-environment jsdom
/**
 * The application entry point (`desktop/src/main.tsx`).
 *
 * This file lives in `src/app/` rather than beside `main.tsx` because the task's
 * write set covers `desktop/src/app/` and `desktop/src/main.tsx` as a *file* — a
 * sibling `desktop/src/main.test.tsx` would be outside it. Placing the test one
 * directory down is equivalent and in scope: `../main` resolves to the same
 * module, and the module's own import of `./app/App` resolves to `./App` here,
 * so the mock below intercepts the same file.
 *
 * The only assertion worth making about an entry point is the one thing it does:
 * resolve `#root` and mount the app into it, under StrictMode. That requires a
 * document which actually owns `#root`, which is why this was previously waived
 * as un-hostable; constructing that document in the test is what removes the
 * waiver.
 */
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("./App", async () => {
  const { createElement: create } = await import("react");
  return {
    App: () => create("div", { "data-child": "app-root" }),
  };
});

describe("main entry point", () => {
  // Importing the entry pulls react-dom/client, the app root and the i18n
  // bundle; on this shared, heavily loaded checkout that cold import exceeds
  // vitest's 5 s default even though nothing blocks. The assertion is unaffected.
  const COLD_IMPORT_MS = 30_000;

  beforeEach(() => {
    // The entry point assumes the real document's shell markup; jsdom starts
    // empty, so provide exactly the element it looks up.
    document.body.innerHTML = "<div id=\"root\"></div>";
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.resetModules();
  });

  it("mounts the app into #root", async () => {
    const root = document.getElementById("root")!;
    // Nothing is mounted yet — the entry point is what puts it there.
    expect(root.childElementCount).toBe(0);

    // Import for its side effect: this is the line the waiver used to cover.
    await import("../main");

    // `createRoot().render()` schedules with concurrent React rather than
    // committing synchronously, so flush the scheduled work before asserting.
    await act(async () => {
      await Promise.resolve();
    });

    // The app is mounted into the shell-owned container, not somewhere else.
    expect(root.querySelector("[data-child=\"app-root\"]")).not.toBeNull();
    expect(document.body.querySelector("[data-child=\"app-root\"]")).not.toBeNull();
    expect(root.childElementCount).toBeGreaterThan(0);
  }, COLD_IMPORT_MS);

  it("throws from #root being absent rather than mounting nowhere", async () => {
    // The entry point casts the lookup (`as HTMLElement`), so a document without
    // `#root` must fail loudly at mount time instead of silently rendering into
    // a detached node. This is the failure a broken index.html would produce.
    document.getElementById("root")!.remove();

    await expect(import("../main")).rejects.toThrow();
  }, COLD_IMPORT_MS);
});
