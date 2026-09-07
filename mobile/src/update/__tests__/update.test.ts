import { Linking, Platform } from "react-native";
import {
  buildChannel,
  checkAndroidUpdate,
  checkIosUpdate,
  compareVersions,
  installUpdate,
  manifestUrl,
  NIGHTLY_MANIFEST_URL,
  RELEASE_MANIFEST_URL,
  shouldOfferAndroidUpdate,
} from "../update";

function mockFetch(body: unknown, ok = true): jest.Mock {
  return jest.fn(async () => ({
    ok,
    status: ok ? 200 : 500,
    json: async () => body,
  }));
}

const androidManifest = (overrides: Record<string, unknown> = {}) => ({
  version: "1.1.0",
  assets: { android: { url: "https://dl.future-os.cn/releases/1.1.0/FutureOS_1.1.0.apk" } },
  ...overrides,
});

const nightlyManifest = (run: number) => ({
  version: `0.0.2-${run}+nightly`,
  assets: {
    android: {
      url: `https://dl.future-os.cn/nightly/0.0.2-${run}/FutureOS_0.0.2-${run}_android-universal.apk`,
    },
  },
});

describe("build channel", () => {
  test.each([
    ["1.0.0", "release"],
    ["0.0.2-100+test", "test"],
    ["0.0.2-100+nightly", "nightly"],
    ["0.0.2-abcdef+dev", "dev"],
    ["0.0.2-abcdef+local", "local"],
    ["0.0.2-abcdef+local.dirty", "local"],
    ["0.0.2-abcdef", "dev"],
  ])("classifies %s as %s", (version, expected) => {
    expect(buildChannel(version)).toBe(expected);
  });

  test("selects exactly one manifest from the current version", () => {
    expect(manifestUrl("1.0.0")).toBe(RELEASE_MANIFEST_URL);
    expect(manifestUrl("0.0.2-100+test")).toBe(NIGHTLY_MANIFEST_URL);
    expect(manifestUrl("0.0.2-100+nightly")).toBe(NIGHTLY_MANIFEST_URL);
    expect(manifestUrl("0.0.2-abcdef+local")).toBe(NIGHTLY_MANIFEST_URL);
  });
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
    const status = await checkAndroidUpdate("1.0.0", mockFetch({ version: "1.1.0", assets: {} }));
    expect(status.hasUpdate).toBe(false);
    expect(status.downloadUrl).toBeNull();
  });

  test("reports no update when the Android asset lacks a url", async () => {
    const status = await checkAndroidUpdate(
      "1.0.0",
      mockFetch(androidManifest({ assets: { android: {} } })),
    );
    expect(status.hasUpdate).toBe(false);
  });

  test("reports an update from the manifest version and download url", async () => {
    const fetchFn = mockFetch(androidManifest());
    const status = await checkAndroidUpdate("1.0.0", fetchFn);
    expect(fetchFn).toHaveBeenCalledWith(RELEASE_MANIFEST_URL);
    expect(status.hasUpdate).toBe(true);
    expect(status.downloadUrl).toBe("https://dl.future-os.cn/releases/1.1.0/FutureOS_1.1.0.apk");
  });

  test("reports no update when already on the latest version", async () => {
    const status = await checkAndroidUpdate("1.1.0", mockFetch(androidManifest()));
    expect(status.hasUpdate).toBe(false);
  });

  test("throws on a non-ok response", async () => {
    await expect(checkAndroidUpdate("1.0.0", mockFetch({}, false))).rejects.toThrow();
  });

  test("test builds upgrade only to a strictly newer nightly run", async () => {
    const older = await checkAndroidUpdate("0.0.2-120+test", mockFetch(nightlyManifest(119)));
    const same = await checkAndroidUpdate("0.0.2-120+test", mockFetch(nightlyManifest(120)));
    const newer = await checkAndroidUpdate("0.0.2-120+test", mockFetch(nightlyManifest(121)));
    expect(older.hasUpdate).toBe(false);
    expect(same.hasUpdate).toBe(false);
    expect(newer.hasUpdate).toBe(true);
    expect(newer.canInstallInApp).toBe(true);
  });

  test("nightly builds never cross to a formal release", () => {
    expect(shouldOfferAndroidUpdate("0.0.2-120+nightly", "1.0.0")).toBe(false);
  });

  test("local and dev builds discover nightlies but remain manual-only", async () => {
    const local = await checkAndroidUpdate("0.0.2-abcdef+local", mockFetch(nightlyManifest(121)));
    const dev = await checkAndroidUpdate("0.0.2-abcdef+dev", mockFetch(nightlyManifest(121)));
    expect(local.hasUpdate).toBe(true);
    expect(local.canInstallInApp).toBe(false);
    expect(dev.hasUpdate).toBe(true);
    expect(dev.canInstallInApp).toBe(false);
  });

  test("manual-only Android updates open the external download URL", async () => {
    const openUrl = jest.spyOn(Linking, "openURL").mockResolvedValueOnce(true);
    const originalPlatform = Platform.OS;
    Object.defineProperty(Platform, "OS", { configurable: true, value: "android" });
    try {
      await installUpdate({
        currentVersion: "0.0.2-abcdef+local",
        latestVersion: "0.0.2-121+nightly",
        hasUpdate: true,
        appStoreUrl: null,
        downloadUrl: "https://dl.future-os.cn/nightly/latest.apk",
        canInstallInApp: false,
      });
      expect(openUrl).toHaveBeenCalledWith("https://dl.future-os.cn/nightly/latest.apk");
    } finally {
      Object.defineProperty(Platform, "OS", { configurable: true, value: originalPlatform });
      openUrl.mockRestore();
    }
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
