import {
  beginNativePresentation,
  endNativePresentation,
  nativePresentationInFlight,
  withNativePresentation,
} from "../nativePresentation";

afterEach(() => {
  // Keep the process-global depth clean for the next test.
  while (nativePresentationInFlight()) endNativePresentation();
});

test("reports a presentation for as long as the caller is waiting", async () => {
  expect(nativePresentationInFlight()).toBe(false);
  let release!: () => void;
  const pending = withNativePresentation(
    () =>
      new Promise<void>(resolve => {
        release = resolve;
      }),
  );
  expect(nativePresentationInFlight()).toBe(true);
  release();
  await pending;
  expect(nativePresentationInFlight()).toBe(false);
});

test("releases on failure so a cancelled picker cannot hold the connection", async () => {
  await expect(
    withNativePresentation(async () => {
      throw new Error("attachment_camera_permission");
    }),
  ).rejects.toThrow("attachment_camera_permission");
  expect(nativePresentationInFlight()).toBe(false);
});

test("nested presentations unwind one at a time", async () => {
  beginNativePresentation();
  beginNativePresentation();
  endNativePresentation();
  expect(nativePresentationInFlight()).toBe(true);
  endNativePresentation();
  expect(nativePresentationInFlight()).toBe(false);
  // An unbalanced release can never drive the counter negative.
  endNativePresentation();
  expect(nativePresentationInFlight()).toBe(false);
});
