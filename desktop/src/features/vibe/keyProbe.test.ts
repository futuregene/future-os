import { describe, expect, it, vi } from "vitest";
import {
  describeKey,
  isGesturePress,
  KEY_PROBE_STORAGE_KEY,
  KEY_PROBE_TAPS,
  KEY_PROBE_WINDOW_MS,
  keyProbeAvailable,
  modifierLabel,
  nextTap,
} from "./keyProbe";

/**
 * The key self-check's rules, as pure steps.
 *
 * The point of these is that the instrument itself cannot lie about its own behaviour:
 * it is off unless asked for, three presses of one unbound chord show it, and the line
 * it prints quotes the values that matter (an empty `code`, `"Unidentified"`, a space).
 */
function press(over: Partial<Parameters<typeof describeKey>[0]> = {}) {
  return {
    alt: false,
    code: "KeyA",
    ctrl: false,
    key: "a",
    meta: false,
    repeat: false,
    shift: false,
    ...over,
  };
}

describe("keyProbeAvailable", () => {
  it("is on in a developer build", () => {
    expect(keyProbeAvailable(true, () => null)).toBe(true);
  });

  it("is off in a packaged build until somebody opts in", () => {
    // A shipped app must behave as if this file did not exist.
    expect(keyProbeAvailable(false, () => null)).toBe(false);
    expect(keyProbeAvailable(false, key => (key === KEY_PROBE_STORAGE_KEY ? "1" : null))).toBe(true);
  });

  it("reads the opt-in from storage when no reader is injected", () => {
    // The shipped path: the default reader is the only thing that touches storage,
    // so an unreadable store has to answer "not available" rather than throw.
    localStorage.removeItem(KEY_PROBE_STORAGE_KEY);
    expect(keyProbeAvailable(false)).toBe(false);
    localStorage.setItem(KEY_PROBE_STORAGE_KEY, "1");
    expect(keyProbeAvailable(false)).toBe(true);
    localStorage.setItem(KEY_PROBE_STORAGE_KEY, "0");
    expect(keyProbeAvailable(false)).toBe(false);
    localStorage.removeItem(KEY_PROBE_STORAGE_KEY);
  });

  it("is off when storage refuses to answer", () => {
    expect(keyProbeAvailable(false, () => {
      throw new Error("denied");
    })).toBe(false);
  });
});

describe("the default storage reader", () => {
  it("answers nothing when storage itself throws", () => {
    // A locked-down webview: reading may throw rather than return null, and the
    // readout simply stays off. The global itself is replaced, not the
    // `Storage` prototype: this file runs in the node environment, where
    // `localStorage` is the suite's in-memory object and `Storage` is not a
    // global at all — so a prototype spy throws `ReferenceError` before it can
    // reach the store the code reads.
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("denied");
      },
    });
    try {
      expect(keyProbeAvailable(false)).toBe(false);
    }
    finally {
      vi.unstubAllGlobals();
    }
  });
});

describe("isGesturePress", () => {
  it("is ⌥⇧K and nothing else", () => {
    expect(isGesturePress(press({ alt: true, shift: true, code: "KeyK" }))).toBe(true);
    // A chord the user is rebinding is not the gesture.
    expect(isGesturePress(press({ alt: true, shift: true, code: "KeyK", meta: true }))).toBe(false);
    expect(isGesturePress(press({ alt: true, shift: true, code: "KeyK", ctrl: true }))).toBe(false);
    expect(isGesturePress(press({ alt: true, code: "KeyK" }))).toBe(false);
    expect(isGesturePress(press({ alt: true, shift: true, code: "KeyJ" }))).toBe(false);
    // Holding the chord is one press, not three: the OS's repeat must not open the panel.
    expect(isGesturePress(press({ alt: true, shift: true, code: "KeyK", repeat: true }))).toBe(false);
  });
});

describe("nextTap", () => {
  const start = { at: 0, taps: 0 };

  it("fires on the third press inside the window", () => {
    let state = nextTap(start, { at: 100, gesture: true });
    expect(state.fired).toBe(false);
    state = nextTap(state, { at: 300, gesture: true });
    expect(state.taps).toBe(2);
    state = nextTap(state, { at: 500, gesture: true });
    expect(state.fired).toBe(true);
    // …and the counter starts over, so a fourth press is the beginning of a new gesture.
    expect(state.taps).toBe(0);
  });

  it("starts over when a press comes too late", () => {
    let state = nextTap(start, { at: 100, gesture: true });
    state = nextTap(state, { at: 100 + KEY_PROBE_WINDOW_MS + 1, gesture: true });
    expect(state.taps).toBe(1);
    expect(state.fired).toBe(false);
  });

  it("is reset by any other key", () => {
    // Otherwise a half-pressed gesture would sit armed while the reader works, and an
    // unrelated press later would open the readout "for no reason".
    let state = nextTap(nextTap(start, { at: 100, gesture: true }), { at: 200, gesture: true });
    expect(state.taps).toBe(2);
    state = nextTap(state, { at: 300, gesture: false });
    expect(state.taps).toBe(0);
    state = nextTap(state, { at: 400, gesture: true });
    expect(state.taps).toBe(1);
    expect(KEY_PROBE_TAPS).toBe(3);
  });
});

describe("describeKey", () => {
  it("prints key, code and modifiers, quoted", () => {
    expect(describeKey(press({ alt: true, shift: true, code: "ArrowUp", key: "ArrowUp" })))
      .toBe(`key="ArrowUp" code="ArrowUp" mods=alt+shift`);
  });

  it("quotes the empty and unprintable values a person cannot eyeball", () => {
    // These are exactly the cases the matrix is being filled in for: a remote's Menu
    // button often sends no usable `code`, and WebKitGTK calls a consumed key
    // "Unidentified". A printed value with no quotes would hide both.
    expect(describeKey(press({ code: "", key: "" }))).toBe(`key="" code="" mods=—`);
    expect(describeKey(press({ code: "", key: "Unidentified" }))).toContain(`key="Unidentified"`);
    expect(describeKey(press({ code: "Space", key: " " }))).toContain(`key=" "`);
  });

  it("marks a repeat, because a remote's held arrow is how one navigates", () => {
    expect(describeKey(press({ code: "ArrowDown", key: "ArrowDown", repeat: true }))).toContain("repeat");
  });

  it("says when no modifier is held", () => {
    expect(modifierLabel({ alt: false, ctrl: false, meta: false, shift: false })).toBe("—");
    expect(modifierLabel({ alt: false, ctrl: true, meta: true, shift: false })).toBe("meta+ctrl");
  });
});
