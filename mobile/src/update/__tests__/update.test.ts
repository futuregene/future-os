import {
  checkAndroidUpdate,
  checkIosUpdate,
  compareVersions,
  UPDATE_MANIFEST_URL,
} from "../update";

function mockFetch(body: unknown, ok = true): jest.Mock {
  return jest.fn(async () => ({
    ok,
    status: ok ? 200 : 500,
    json: async () => body,
  }));
}

const androidManifest = (overrides: Record<string, unknown> = {}) => ({
  version: "1.0.0",
  assets: { android: { url: "https://dl.future-os.cn/releases/1.0.0/FutureOS_1.0.0.apk" } },
  ...overrides,
});

describe("compareVersions", () => {
  test("orders newer releases above older ones", () => {
    expect(compareVersions("1.0.1", "1.0.0")).toBeGreaterThan(0);
    expect(compareVersions("1.0.0", "1.0.1")).toBeLessThan(0);
    expect(compareVersions("1.10.0", "1.9.0")).toBeGreaterThan(0);
  });

  test("treats equal semver cores as equal", () => {
    expect(compareVersions("1.0.0", "1.0.0")).toBe(0);
    expect(compareVersions("1.0", "1.0.0")).toBe(0);
  });

  test("ignores prerelease and build suffixes", () => {
    expect(compareVersions("1.0.0-beta", "1.0.0")).toBe(0);
    expect(compareVersions("0.0.2-abc", "1.0.0")).toBeLessThan(0);
  });
});

describe("checkAndroidUpdate", () => {
  test("reports no update when the manifest has no Android asset", async () => {
    const status = await checkAndroidUpdate("0.9.0", mockFetch({ version: "1.0.0", assets: {} }));
    expect(status.hasUpdate).toBe(false);
    expect(status.downloadUrl).toBeNull();
  });

  test("reports no update when the Android asset lacks a url", async () => {
    const status = await checkAndroidUpdate(
      "0.9.0",
      mockFetch(androidManifest({ assets: { android: {} } })),
    );
    expect(status.hasUpdate).toBe(false);
  });

  test("reports an update from the manifest version and download url", async () => {
    const fetchFn = mockFetch(androidManifest());
    const status = await checkAndroidUpdate("0.9.0", fetchFn);
    expect(fetchFn).toHaveBeenCalledWith(UPDATE_MANIFEST_URL);
    expect(status.hasUpdate).toBe(true);
    expect(status.downloadUrl).toBe("https://dl.future-os.cn/releases/1.0.0/FutureOS_1.0.0.apk");
  });

  test("reports no update when already on the latest version", async () => {
    const status = await checkAndroidUpdate("1.0.0", mockFetch(androidManifest()));
    expect(status.hasUpdate).toBe(false);
  });

  test("throws on a non-ok response", async () => {
    await expect(checkAndroidUpdate("0.9.0", mockFetch({}, false))).rejects.toThrow();
  });
});

describe("checkIosUpdate", () => {
  test("reports no update when the app is not on the App Store", async () => {
    const status = await checkIosUpdate("0.9.0", mockFetch({ resultCount: 0, results: [] }));
    expect(status.hasUpdate).toBe(false);
    expect(status.appStoreUrl).toBeNull();
  });

  test("reports an update from the App Store version", async () => {
    const status = await checkIosUpdate(
      "1.0.0",
      mockFetch({
        resultCount: 1,
        results: [{ version: "1.1.0", trackViewUrl: "https://apps.apple.com/app/id123" }],
      }),
    );
    expect(status.hasUpdate).toBe(true);
    expect(status.appStoreUrl).toBe("https://apps.apple.com/app/id123");
  });

  test("reports no update when the App Store version is current", async () => {
    const status = await checkIosUpdate(
      "1.0.0",
      mockFetch({
        resultCount: 1,
        results: [{ version: "1.0.0", trackViewUrl: "https://apps.apple.com/app/id123" }],
      }),
    );
    expect(status.hasUpdate).toBe(false);
  });
});
