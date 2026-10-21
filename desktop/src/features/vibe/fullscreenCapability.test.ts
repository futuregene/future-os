import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * The capability file is a runtime dependency of the panel's fullscreen button, and the
 * one place where "the code is right and the app still does nothing" can happen with no
 * test failing — which is exactly what happened: the button called `setFullscreen`, the
 * capability layer rejected it, `.catch(() => {})` swallowed the rejection, and the release
 * shipped a button that did nothing.
 *
 * So the grant is asserted here, next to the code that needs it. `isFullscreen` comes from
 * `core:default` and was never the problem; `setFullscreen` is an explicit grant and must
 * stay one.
 */
const capabilities = JSON.parse(
  readFileSync(resolve(__dirname, "../../../src-tauri/capabilities/default.json"), "utf8"),
) as { permissions: string[]; windows: string[] };

describe("fullscreen capability", () => {
  it("grants set_fullscreen to the main window", () => {
    expect(capabilities.windows).toContain("main");
    expect(capabilities.permissions).toContain("core:window:allow-set-fullscreen");
  });

  it("keeps the drag permission the panel's bar also uses", () => {
    // The Vibe panel's bar is the only draggable surface left while the panel covers the
    // window, and it starts a drag through the same window API.
    expect(capabilities.permissions).toContain("core:window:allow-start-dragging");
  });

  it("does not deny what it grants", () => {
    // A deny entry would win over the allow, and the button would go silent again.
    for (const permission of capabilities.permissions)
      expect(permission.startsWith("core:window:deny-"), permission).toBe(false);
  });
});
