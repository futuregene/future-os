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
// integers. CI injects FUTURE_BUILD_NUMBER (this workflow's own per-repo
// counter, starting at 1 — kept small because Android versionCode is an int32);
// local dev builds just use a constant — TestFlight/Play enforce monotonicity
// only, not that the number corresponds to any git commit.
const buildNumber = process.env.FUTURE_BUILD_NUMBER || "1";
const cameraPermission =
  "FutureOS uses the camera to scan desktop pairing QR codes and take photos you choose to attach to conversations.";

const config: ExpoConfig = {
  name: "FutureOS",
  slug: "futureos",
  scheme: "futureos",
  version,
  orientation: "portrait",
  // iOS requires an opaque, full-bleed 1024px source. The desktop icon keeps
  // transparent padding for its own use, so use the mobile-specific crop here.
  icon: "./assets/icon.png",
  userInterfaceStyle: "light",
  plugins: [
    "./plugins/withIosShareExtension",
    // Expo CLI cannot add this automatically to a dynamic TypeScript config.
    // With no options it retains expo-sharing's disabled share-in extensions;
    // FutureOS's optional inbound extension remains owned by the plugin above.
    "expo-sharing",
    [
      "expo-camera",
      {
        cameraPermission,
        recordAudioAndroid: false,
      },
    ],
    [
      "expo-image-picker",
      {
        cameraPermission,
        photosPermission:
          "Allow FutureOS to select photos for conversation attachments.",
        microphonePermission: false,
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
    // Keep the local-notification module, but do not request APNs push access.
    // Entitlement mods execute in reverse registration order: register cleanup
    // first so it removes APNs after expo-notifications has added it.
    "./plugins/withLocalNotificationsOnly",
    "expo-notifications",
    [
      "expo-splash-screen",
      {
        // Match the app's first-frame canvas color so the launch screen fades
        // into the booting spinner with no visible seam. Android 12+ requires a
        // splash icon (it defaults to the launcher icon if none is set), so we
        // pass a fully transparent image — visually a bare background on both
        // platforms, no logo.
        backgroundColor: "#f6f7f9",
        image: "./assets/splash.png",
      },
    ],
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
          // Xcode 27 / iOS 27 rejects the legacy UIApplication-only lifecycle
          // at launch. Expo SDK 57 can opt into its backported scene delegate
          // until the project moves to SDK 58, where scenes are the default.
          enableSceneSupport: true,
        },
      },
    ],
  ],
  ios: {
    bundleIdentifier: "cn.futureos.mobile",
    buildNumber,
    supportsTablet: true,
    // React strings use expo-localization, but UIKit's editing menu (Paste,
    // AutoFill, etc.) chooses its language from the native bundle's declared
    // localizations. Declare the Chinese variants here so those system controls
    // follow a Chinese device/app language as well.
    infoPlist: {
      CFBundleLocalizations: ["en", "zh-Hans", "zh-Hant"],
      // Open In is independent of the optional Share Extension/App Group.
      // Import a private copy; never edit the source document in place.
      LSSupportsOpeningDocumentsInPlace: false,
      CFBundleDocumentTypes: [{
        CFBundleTypeName: "Documents and images",
        CFBundleTypeRole: "Viewer",
        LSHandlerRank: "Alternate",
        LSItemContentTypes: [
          "public.image", "public.text", "com.adobe.pdf",
          "com.microsoft.word.doc", "org.openxmlformats.wordprocessingml.document",
          "com.microsoft.powerpoint.ppt", "org.openxmlformats.presentationml.presentation",
          "com.microsoft.excel.xls", "org.openxmlformats.spreadsheetml.sheet",
          "net.daringfireball.markdown",
        ],
      }],
      UTImportedTypeDeclarations: [
        ["net.daringfireball.markdown", ["md", "markdown"], "text/markdown"],
        ["com.microsoft.word.doc", ["doc"], "application/msword"],
        ["org.openxmlformats.wordprocessingml.document", ["docx"], "application/vnd.openxmlformats-officedocument.wordprocessingml.document"],
        ["com.microsoft.powerpoint.ppt", ["ppt"], "application/vnd.ms-powerpoint"],
        ["org.openxmlformats.presentationml.presentation", ["pptx"], "application/vnd.openxmlformats-officedocument.presentationml.presentation"],
        ["com.microsoft.excel.xls", ["xls"], "application/vnd.ms-excel"],
        ["org.openxmlformats.spreadsheetml.sheet", ["xlsx"], "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"],
      ].map(([identifier, extensions, mime]) => ({
        UTTypeIdentifier: identifier,
        UTTypeConformsTo: [identifier === "net.daringfireball.markdown" ? "public.plain-text" : "public.data"],
        UTTypeTagSpecification: {
          "public.filename-extension": extensions,
          "public.mime-type": mime,
        },
      })),
    },
    // Required by expo-secure-store. Keep this in the Expo config because the
    // ios directory is generated by prebuild and is not version-controlled.
    entitlements: {
      "keychain-access-groups": ["$(AppIdentifierPrefix)$(CFBundleIdentifier)"],
    },
    config: {
      usesNonExemptEncryption: false,
    },
  },
  android: {
    package: "cn.futureos.mobile",
    versionCode: Number.parseInt(buildNumber, 10),
    permissions: ["android.permission.REQUEST_INSTALL_PACKAGES"],
    // The notifications module also brings optional FCM remote-push support.
    // FutureOS has local task reminders only, so keep its display/recovery
    // permissions while blocking the remote-push and unrelated overlay access.
    blockedPermissions: [
      "android.permission.SYSTEM_ALERT_WINDOW",
      "android.permission.WAKE_LOCK",
      "com.google.android.c2dm.permission.RECEIVE",
    ],
    // Share targets. `MainActivity` is `singleTask`, so a share while the app
    // is running is delivered through `onNewIntent` (see the local
    // `future-share-intent` module, which captures both receipts).
    // TEXT is listed alongside the file types because many apps send shared
    // text with `*/*`, and a mimeType filter for `text/plain` alone would drop
    // those. Images keep their own entry so the gallery offers FutureOS first.
    intentFilters: [
      {
        action: "VIEW",
        category: ["DEFAULT"],
        // Modern file providers grant content:// access. Do not claim web or
        // futureos:// links, or accept arbitrary paths into our private files.
        data: [
          "application/pdf", "application/msword",
          "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
          "application/vnd.ms-powerpoint",
          "application/vnd.openxmlformats-officedocument.presentationml.presentation",
          "application/vnd.ms-excel",
          "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
          "text/*", "image/*", "application/octet-stream",
        ].map(mimeType => ({ scheme: "content", mimeType })),
      },
      {
        action: "SEND",
        category: ["DEFAULT"],
        data: [{ mimeType: "text/plain" }],
      },
      {
        action: "SEND",
        category: ["DEFAULT"],
        data: [{ mimeType: "image/*" }],
      },
      {
        action: "SEND_MULTIPLE",
        category: ["DEFAULT"],
        data: [{ mimeType: "image/*" }],
      },
      { action: "SEND", category: ["DEFAULT"], data: [{ mimeType: "*/*" }] },
      {
        action: "SEND_MULTIPLE",
        category: ["DEFAULT"],
        data: [{ mimeType: "*/*" }],
      },
    ],
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
