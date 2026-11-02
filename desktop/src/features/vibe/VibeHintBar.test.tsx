// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { VibeHintBar } from "./VibeHintBar";

/**
 * The panel's hint line.
 *
 * Two sources feed one line: keys the panel shows but does not own (`fixed` — the
 * composer's own ⏎) and the bound commands of the layer that is open. The line is
 * generated so it cannot drift from the keyboard; what this file holds is that the
 * fixed half is *rendered*, since a hint nobody can see is the drift it exists to
 * prevent.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

async function mount(fixed: readonly { code: string; labelKey: string }[], ids: Parameters<typeof VibeHintBar>[0]["ids"]) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<VibeHintBar fixed={fixed} ids={ids} />);
  });
  return {
    container,
    text: () => container.textContent ?? "",
    unmount: async () => {
      await act(async () => root.unmount());
      container.remove();
    },
  };
}

describe("vibe hint bar", () => {
  it("shows a fixed key with its own label", async () => {
    const view = await mount([{ code: "Enter", labelKey: "fixed.send" }], []);

    // The key's glyph and the sentence the settings page uses for it.
    expect(view.container.querySelector("kbd")).not.toBeNull();
    expect(view.text()).toContain("Send the message");
    await view.unmount();
  });

  it("draws nothing when the layer promises no keys", async () => {
    const view = await mount([], []);
    expect(view.container.textContent).toBe("");
    await view.unmount();
  });
});
