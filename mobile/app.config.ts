import type { ExpoConfig } from "expo/config";
import { execFileSync } from "node:child_process";

function futureVersion(): string {
  return execFileSync(process.execPath, ["../scripts/version.mjs"], {
    cwd: __dirname,
    encoding: "utf8",
  }).trim();
}

const version = futureVersion();
const bundleVersion = version.split(/[-+]/)[0];
// Store build numbers (CFBundleVersion / Android versionCode) must be monotonic
// integers. CI injects FUTURE_BUILD_NUMBER (a repo-wide counter that never
// resets); local dev builds just use a constant — TestFlight/Play enforce
// monotonicity only, not that the number corresponds to any git commit.
const buildNumber = process.env.FUTURE_BUILD_NUMBER || "1";

const config: ExpoConfig = {
  name: "FutureOS",
  slug: "futureos",
  scheme: "futureos",
  version,
  orientation: "portrait",
  icon: "../desktop/src-tauri/icons/icon.png",
  userInterfaceStyle: "light",
  plugins: [
    [
      "expo-camera",
      {
        cameraPermission: "Allow FutureOS to scan the desktop pairing QR code.",
        recordAudioAndroid: false,
      },
    ],
    [
      "expo-secure-store",
      {
        configureAndroidBackup: true,
        faceIDPermission: "Allow FutureOS to unlock remote credentials.",
      },
    ],
    "expo-localization",
    [
      "expo-build-properties",
      {
        android: {
          compileSdkVersion: 36,
          targetSdkVersion: 36,
          buildToolsVersion: "36.0.0",
        },
        ios: {
          deploymentTarget: "16.4",
        },
      },
    ],
  ],
  ios: {
    bundleIdentifier: "cn.futureos.mobile",
    buildNumber,
    supportsTablet: true,
    config: {
      usesNonExemptEncryption: false,
    },
  },
  android: {
    package: "cn.futureos.mobile",
    versionCode: Number.parseInt(buildNumber, 10),
    adaptiveIcon: {
      backgroundColor: "#0f172a",
      foregroundImage: "../desktop/src-tauri/icons/icon.png",
    },
    predictiveBackGestureEnabled: true,
  },
  extra: {
    futureVersion: version,
    bundleVersion,
    developmentPlatformUrl: "https://test.future-os.cn",
    productionPlatformUrl: "https://future-os.cn",
  },
};

export default config;
