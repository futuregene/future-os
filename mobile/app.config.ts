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

const config: ExpoConfig = {
  name: "FutureOS",
  slug: "future-os-mobile",
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
    bundleIdentifier: "cn.future_os.mobile",
    buildNumber: "1",
    supportsTablet: true,
    config: {
      usesNonExemptEncryption: false,
    },
  },
  android: {
    package: "cn.future_os.mobile",
    versionCode: 1,
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
