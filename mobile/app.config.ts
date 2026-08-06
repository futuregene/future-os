import type { ExpoConfig } from "expo/config";
import { execFileSync } from "node:child_process";

function futureVersion(): string {
  return execFileSync(process.execPath, ["../scripts/version.mjs"], {
    cwd: __dirname,
    encoding: "utf8",
  }).trim();
}

function gitCommitCount(): string {
  return execFileSync("git", ["rev-list", "--count", "HEAD"], {
    cwd: __dirname,
    encoding: "utf8",
  }).trim();
}

const version = futureVersion();
const bundleVersion = version.split(/[-+]/)[0];
// Store build numbers must be monotonic integers. CI injects FUTURE_BUILD_NUMBER
// (the git commit count); local builds derive it the same way. TestFlight/app
// stores reject the `-<hash>` suffix, so this stays a plain number.
const buildNumber = process.env.FUTURE_BUILD_NUMBER || gitCommitCount();

const config: ExpoConfig = {
  name: "FutureOS",
  slug: "futureos",
  scheme: "futureos",
  version,
  orientation: "portrait",
  icon: "../gui/src-tauri/icons/icon.png",
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
          usesCleartextTraffic: true,
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
      foregroundImage: "../gui/src-tauri/icons/icon.png",
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
