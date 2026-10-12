import { expect, it } from "vitest";
import { windowRoute } from "./windowRoute";

/**
 * `windowRoute` decides which page a *native window* renders, so both mistakes
 * are quiet: the main window showing a settings page for a machine nobody opened,
 * or a settings window rendering the whole app shell (leaving that machine's
 * settings unreachable).
 */

it("routes an unmarked window to the app", () => {
  expect(windowRoute("")).toEqual({ kind: "app" });
  expect(windowRoute("?desktop=desktop_a")).toEqual({ kind: "app" });
  expect(windowRoute("?window=something-else&desktop=desktop_a")).toEqual({ kind: "app" });
});

it("routes the settings window to its machine", () => {
  expect(windowRoute("?window=peer-settings&desktop=desktop_a")).toEqual({
    kind: "peer-settings",
    desktopId: "desktop_a",
  });
});

/**
 * A settings window with no machine has nothing to show. Rendering the app shell
 * is the honest fallback — the alternative is a settings page addressed to
 * nobody, which would write to whatever id it happened to hold.
 */
it("falls back to the app when the machine is missing or empty", () => {
  expect(windowRoute("?window=peer-settings")).toEqual({ kind: "app" });
  expect(windowRoute("?window=peer-settings&desktop=")).toEqual({ kind: "app" });
});

/**
 * The backend percent-encodes the id (an `&` in it would otherwise end the
 * parameter), so the parser has to decode it back — otherwise the page reads a
 * machine id the store does not have.
 */
it("decodes an encoded machine id", () => {
  expect(windowRoute("?window=peer-settings&desktop=a%26b%3Dc")).toEqual({
    kind: "peer-settings",
    desktopId: "a&b=c",
  });
});
