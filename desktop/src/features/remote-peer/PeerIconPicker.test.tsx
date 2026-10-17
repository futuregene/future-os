// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PeerIconPicker } from "./PeerIconPicker";
import { PEER_ICONS } from "./peerIcons";

/**
 * Picking the glyph a paired machine is drawn with.
 *
 * The picker is a radio group: exactly one icon is the machine's current one, and
 * choosing another reports it. Both halves matter — a picker that reports a click
 * but never marks the current icon leaves the user unable to tell what the
 * machine is set to, and one that marks it but reports nothing is decoration.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  document.body.innerHTML = "";
});

function mount(value: string) {
  const onChange = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<PeerIconPicker onChange={onChange} value={value} />));
  const radios = () => [...container.querySelectorAll<HTMLButtonElement>("[role='radio']")];
  return {
    checked: () => radios().filter(radio => radio.getAttribute("aria-checked") === "true").map(radio => radio.getAttribute("title")),
    container,
    onChange,
    radios,
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

describe("peerIconPicker", () => {
  it("offers every icon, with the machine's own one marked", () => {
    const view = mount("laptop");
    try {
      const radios = view.radios();
      expect(radios).toHaveLength(PEER_ICONS.length);
      // One group, named by the fieldset's own label rather than by a glyph.
      expect(view.container.querySelector("[aria-label]")).not.toBeNull();
      expect(view.checked()).toHaveLength(1);
      // The marked one is the current value, not merely the first icon.
      expect(radios.find(radio => radio.getAttribute("aria-checked") === "true")!.title)
        .toBe("Laptop");
    }
    finally {
      view.unmount();
    }
  });

  it("reports the icon the user picked, by id", () => {
    const view = mount("laptop");
    try {
      const phone = view.radios().find(radio => radio.title === "Phone")!;
      act(() => phone.click());

      expect(view.onChange).toHaveBeenCalledExactlyOnceWith("phone");
      // Reported, not applied: the value is the caller's, so the picker still
      // shows what it was given.
      expect(view.checked()).toEqual(["Laptop"]);
    }
    finally {
      view.unmount();
    }
  });

  it("marks nothing when the machine's icon is not one of them", () => {
    // A machine paired by an older build can carry an id this list no longer
    // has. Then no icon is current — marking the first one would claim a glyph
    // the host is not drawn with.
    const view = mount("not-an-icon");
    try {
      expect(view.checked()).toEqual([]);
      expect(view.radios()).toHaveLength(PEER_ICONS.length);
    }
    finally {
      view.unmount();
    }
  });
});
