import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearVibePreferences,
  readVibePreferences,
  writeVibeColumns,
  writeVibeDevice,
  writeVibeOrder,
  writeVibePanel,
} from "./vibePreferences";

/**
 * The panel's stored preferences are *untrusted input*: an older build wrote them,
 * a person may have hand-edited them, and a write may have been cut short. These
 * tests pin the contract the readers promise — every field is validated, and a bad
 * value falls back to the documented default instead of being guessed at — which is
 * also the only way to reach the fallback arms at all (`localStorage` only ever
 * holds what a previous read-modify-write wrote).
 */
const PREFS_KEY = "future.vibe.prefs";
const ORDER_KEY = "future.vibe.order";

function storePrefs(value: unknown): void {
  localStorage.setItem(PREFS_KEY, typeof value === "string" ? value : JSON.stringify(value));
}

beforeEach(() => {
  clearVibePreferences();
});

describe("reading an empty profile", () => {
  it("answers the documented defaults when nothing was ever stored", () => {
    expect(readVibePreferences()).toEqual({
      columns: null,
      device: { kind: "all" },
      order: [],
      panel: {},
      panelDefault: "auto",
    });
  });
});

describe("reading damage rather than trusting it", () => {
  it("drops both documents when the JSON is unparseable", () => {
    localStorage.setItem(PREFS_KEY, "{not json");
    localStorage.setItem(ORDER_KEY, "{not json");
    expect(readVibePreferences()).toEqual(
      expect.objectContaining({ columns: null, device: { kind: "all" }, order: [], panel: {}, panelDefault: "auto" }),
    );
  });

  it("treats a document that is not an object as absent", () => {
    // `JSON.parse` accepts these; only a shape check rejects them.
    for (const shape of ["null", "\"a string\"", "42", "[]"]) {
      storePrefs(shape);
      localStorage.setItem(ORDER_KEY, shape);
      expect(readVibePreferences(), shape).toEqual(
        expect.objectContaining({ columns: null, device: { kind: "all" }, order: [], panel: {}, panelDefault: "auto" }),
      );
    }
  });

  it("rejects an order that is not an array of panel keys", () => {
    for (const order of ["\"a string\"", "{}", "5"]) {
      localStorage.setItem(ORDER_KEY, order);
      expect(readVibePreferences().order, order).toEqual([]);
    }
  });

  it("keeps only string entries that name a panel", () => {
    localStorage.setItem(ORDER_KEY, JSON.stringify(["thread:a", 7, "nocolon", { id: "x" }, "remote:d:s"]));
    expect(readVibePreferences().order).toEqual(["thread:a", "remote:d:s"]);
  });

  it("rejects a device filter it cannot interpret", () => {
    storePrefs({ device: null });
    expect(readVibePreferences().device).toEqual({ kind: "all" });
    storePrefs({ device: "desktop" });
    expect(readVibePreferences().device).toEqual({ kind: "all" });
    // A machine id without a usable id is not a filter.
    storePrefs({ device: { desktopId: "", kind: "device" } });
    expect(readVibePreferences().device).toEqual({ kind: "all" });
    // An unknown kind is dropped rather than treated as "all devices".
    storePrefs({ device: { kind: "everything" } });
    expect(readVibePreferences().device).toEqual({ kind: "all" });
  });
});

describe("reading each field on its own", () => {
  it("keeps an explicit device only with a machine id", () => {
    storePrefs({ device: { desktopId: "desk-1", kind: "device" } });
    expect(readVibePreferences().device).toEqual({ desktopId: "desk-1", kind: "device" });
    storePrefs({ device: { kind: "all" } });
    expect(readVibePreferences().device).toEqual({ kind: "all" });
  });

  it("accepts only a positive integer column count", () => {
    storePrefs({ columns: 3 });
    expect(readVibePreferences().columns).toBe(3);
    for (const columns of [0, -2, 2.5, "3", null]) {
      storePrefs({ columns });
      expect(readVibePreferences().columns, String(columns)).toBeNull();
    }
  });

  it("ignores a panel map that is not an object, and passes a real one through", () => {
    storePrefs({ panel: "files" });
    expect(readVibePreferences().panel).toEqual({});
    storePrefs({ panel: { "thread:a": "pinned" } });
    expect(readVibePreferences().panel).toEqual({ "thread:a": "pinned" });
  });

  it("reads only `hidden` as a hidden default", () => {
    storePrefs({ panelDefault: "hidden" });
    expect(readVibePreferences().panelDefault).toBe("hidden");
    for (const panelDefault of ["auto", "pinned", 1, null]) {
      storePrefs({ panelDefault });
      expect(readVibePreferences().panelDefault, String(panelDefault)).toBe("auto");
    }
  });
});

describe("writing keeps the other fields", () => {
  it("merges a column write into the stored document instead of replacing it", () => {
    storePrefs({ panel: { "thread:a": "pinned" }, panelDefault: "hidden" });
    writeVibeColumns(2);
    expect(readVibePreferences()).toEqual(
      expect.objectContaining({ columns: 2, panel: { "thread:a": "pinned" }, panelDefault: "hidden" }),
    );
  });

  it("writes a panel choice and its sticky default in one document", () => {
    writeVibePanel("thread:b", "files", "hidden");
    expect(readVibePreferences()).toEqual(
      expect.objectContaining({ panel: { "thread:b": "files" }, panelDefault: "hidden" }),
    );
  });

  it("remembers the device filter", () => {
    writeVibeDevice({ desktopId: "desk-9", kind: "device" });
    expect(readVibePreferences().device).toEqual({ desktopId: "desk-9", kind: "device" });
    writeVibeDevice({ kind: "all" });
    expect(readVibePreferences().device).toEqual({ kind: "all" });
  });

  it("remembers the order", () => {
    writeVibeOrder(["thread:a", "thread:b"]);
    expect(readVibePreferences().order).toEqual(["thread:a", "thread:b"]);
  });

  it("survives storage that refuses to write", () => {
    // A quota-limited or blocked storage must cost the user nothing but the
    // preference surviving a restart — the call itself may not throw.
    const setItem = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("QuotaExceededError");
    });
    expect(() => writeVibeColumns(2)).not.toThrow();
    expect(() => writeVibeOrder(["thread:a"])).not.toThrow();
    setItem.mockRestore();
  });
});
