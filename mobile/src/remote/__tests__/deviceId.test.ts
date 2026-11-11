import { shortDeviceId } from "../deviceId";

describe("shortDeviceId", () => {
  test("keeps the dev_ prefix and the first eight characters of the body", () => {
    expect(shortDeviceId("dev_1a2b3c4d5e6f7a8b")).toBe("dev_1a2b3c4d");
  });

  // The platform always issues a `dev_`-prefixed id, so any other shape is a
  // fixture or another implementation. It still has to render as something
  // stable rather than throwing on the settings page.
  test("shortens an id of an unexpected shape without guessing at it", () => {
    expect(shortDeviceId(" 1a2b3c4d5e6f7a8b ")).toBe("dev_1a2b3c4d");
    expect(shortDeviceId("phone")).toBe("dev_phone");
  });
});
