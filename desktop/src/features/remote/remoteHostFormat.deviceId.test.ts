import type { TFunction } from "i18next";
import { describe, expect, it } from "vitest";
import { deviceLabel, shortDeviceId } from "./remoteHostFormat";

/**
 * The same key-and-interpolation stub the version tests use: these helpers
 * choose *whether* to say something, and the sentence is the translator's.
 */
const t = ((key: string, options?: Record<string, unknown>) => {
  const values = Object.entries(options ?? {})
    .map(([name, value]) => `${name}=${String(value)}`)
    .join(",");
  return values ? `${key}(${values})` : key;
}) as unknown as TFunction<"remote">;

function phone(deviceId?: string | null) {
  return {
    connected: true,
    client: { name: "PJZ110", kind: "mobile", deviceId },
  };
}

describe("which phone is connected", () => {
  // Two phones of one model report the same name and kind. Before the id rode
  // the presence beat, their rows were the same words and the person looking at
  // the page had no way to tell which one was in their hand. Remove the id from
  // what the client declares and this test must fail.
  it("tells two devices of the same model apart", () => {
    const first = deviceLabel(phone("dev_1a2b3c4d5e6f7a8b"), t);
    const second = deviceLabel(phone("dev_9f8e7d6c5b4a3928"), t);
    expect(first).not.toBe(second);
    expect(first).toBe("clientKind.mobile(defaultValue=clientKind.other,name=PJZ110) · dev_1a2b3c4d");
    expect(second).toBe("clientKind.mobile(defaultValue=clientKind.other,name=PJZ110) · dev_9f8e7d6c");
  });

  // The id is a lookup key, not something this page puts on screen in full: the
  // short form is what a person compares against the phone, and the long one
  // would be copied off the machine by anyone who walks past it.
  it("prints only the short form of the id", () => {
    const label = deviceLabel(phone("dev_1a2b3c4d5e6f7a8b"), t);
    expect(label).toContain(shortDeviceId("dev_1a2b3c4d5e6f7a8b"));
    expect(label).not.toContain("dev_1a2b3c4d5e6f7a8b");
  });

  // Every client that predates the field sends none, and a device that named
  // itself without one must read exactly as it did before: no separator
  // dangling after the name, no "undefined" for the reader.
  it("leaves the label untouched when the device declared no id", () => {
    for (const missing of [undefined, null, "", "   "]) {
      const label = deviceLabel(phone(missing), t);
      expect({ missing, label }).toEqual({
        missing,
        label: "clientKind.mobile(defaultValue=clientKind.other,name=PJZ110)",
      });
      expect(label).not.toContain("undefined");
      expect(label).not.toContain("·");
    }
  });
});

describe("shortDeviceId", () => {
  it("keeps the dev_ prefix and the first eight characters of the body", () => {
    expect(shortDeviceId("dev_00112233445566778899")).toBe("dev_00112233");
  });

  it("shortens an id of an unexpected shape without guessing at it", () => {
    expect(shortDeviceId("00112233445566778899")).toBe("dev_00112233");
    expect(shortDeviceId("phone")).toBe("dev_phone");
  });

  it("has nothing to say about an absent id", () => {
    expect(shortDeviceId(undefined)).toBe("");
    expect(shortDeviceId(null)).toBe("");
    expect(shortDeviceId("   ")).toBe("");
    expect(shortDeviceId("dev_")).toBe("dev_");
  });
});
