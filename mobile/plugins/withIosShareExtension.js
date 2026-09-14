const { withEntitlementsPlist, withInfoPlist, withXcodeProject } = require("@expo/config-plugins");
const fs = require("node:fs");
const path = require("node:path");
const plist = require("@expo/plist").default;

const TARGET = "FutureShareExtension";
const GROUP = "group.cn.futureos.mobile.share";
const BUNDLE = "cn.futureos.mobile.share";
const unquote = value => String(value).replace(/^"|"$/g, "");

// Kept in a prebuild plugin: mobile/ios is generated and must not be committed.
function configureProject(project, { platformRoot, moduleRoot, version, buildNumber, manualSigning }) {
  const directory = path.join(platformRoot, TARGET);
  fs.mkdirSync(directory, { recursive: true });
  fs.copyFileSync(path.join(moduleRoot, "ios", "ShareInbox.swift"), path.join(directory, "ShareInbox.swift"));
  fs.copyFileSync(path.join(moduleRoot, "extension", "ShareViewController.swift"), path.join(directory, "ShareViewController.swift"));
  fs.writeFileSync(path.join(directory, `${TARGET}.entitlements`), plist.build({
    "com.apple.security.application-groups": [GROUP],
  }));
  fs.writeFileSync(path.join(directory, `${TARGET}-Info.plist`), plist.build({
    CFBundleDisplayName: "FutureOS",
    CFBundleExecutable: "$(EXECUTABLE_NAME)",
    CFBundleIdentifier: "$(PRODUCT_BUNDLE_IDENTIFIER)",
    CFBundleInfoDictionaryVersion: "6.0",
    CFBundleName: "$(PRODUCT_NAME)",
    CFBundlePackageType: "XPC!",
    CFBundleShortVersionString: "$(MARKETING_VERSION)",
    CFBundleVersion: "$(CURRENT_PROJECT_VERSION)",
    CFBundleLocalizations: ["en", "zh-Hans", "zh-Hant"],
    FutureShareAppGroup: GROUP,
    NSExtension: {
      NSExtensionPointIdentifier: "com.apple.share-services",
      NSExtensionPrincipalClass: "$(PRODUCT_MODULE_NAME).ShareViewController",
      NSExtensionAttributes: {
        NSExtensionActivationRule: {
          NSExtensionActivationSupportsText: true,
          NSExtensionActivationSupportsWebURLWithMaxCount: 10,
          NSExtensionActivationSupportsImageWithMaxCount: 10,
          NSExtensionActivationSupportsFileWithMaxCount: 10,
        },
      },
    },
  }));

  // node-xcode expects these sections even in a template without dependencies.
  project.hash.project.objects.PBXTargetDependency ??= {};
  project.hash.project.objects.PBXContainerItemProxy ??= {};
  let target = Object.entries(project.pbxNativeTargetSection())
    .find(([, value]) => typeof value === "object" && unquote(value.name) === TARGET);
  if (!target) {
    const added = project.addTarget(TARGET, "app_extension", TARGET, BUNDLE);
    target = [added.uuid, added.pbxNativeTarget];
    project.addBuildPhase([], "PBXSourcesBuildPhase", "Sources", added.uuid);
    project.addBuildPhase([], "PBXFrameworksBuildPhase", "Frameworks", added.uuid);
    const group = project.addPbxGroup([], TARGET, TARGET);
    project.addToPbxGroup(group.uuid, project.getFirstProject().firstProject.mainGroup);
    for (const file of ["ShareInbox.swift", "ShareViewController.swift"]) {
      project.addSourceFile(file, { target: added.uuid }, group.uuid);
    }
    for (const file of [`${TARGET}-Info.plist`, `${TARGET}.entitlements`]) {
      project.addFile(file, group.uuid);
    }
  }
  for (const file of Object.values(project.hash.project.objects.PBXBuildFile)) {
    if (typeof file === "object" && file.fileRef === target[1].productReference) {
      file.settings = { ATTRIBUTES: ["RemoveHeadersOnCopy"] };
    }
  }
  const configurations = project.pbxXCBuildConfigurationSection();
  const lists = project.pbxXCConfigurationList();
  for (const reference of lists[target[1].buildConfigurationList].buildConfigurations) {
    const settings = configurations[reference.value].buildSettings;
    Object.assign(settings, {
      PRODUCT_BUNDLE_IDENTIFIER: `"${BUNDLE}"`,
      CODE_SIGN_ENTITLEMENTS: `"${TARGET}/${TARGET}.entitlements"`,
      IPHONEOS_DEPLOYMENT_TARGET: "16.4",
      SWIFT_VERSION: "5.0",
      TARGETED_DEVICE_FAMILY: '"1,2"',
      APPLICATION_EXTENSION_API_ONLY: "YES",
      GENERATE_INFOPLIST_FILE: "NO",
      MARKETING_VERSION: `"${version}"`,
      CURRENT_PROJECT_VERSION: `"${buildNumber}"`,
    });
    if (manualSigning) {
      settings.CODE_SIGN_STYLE = "Manual";
      settings.PROVISIONING_PROFILE_SPECIFIER = '"$(FUTURE_SHARE_PROFILE)"';
    }
  }
  if (manualSigning) {
    const app = project.getFirstTarget().firstTarget;
    for (const ref of lists[app.buildConfigurationList].buildConfigurations) {
      configurations[ref.value].buildSettings.CODE_SIGN_STYLE = "Manual";
      configurations[ref.value].buildSettings.PROVISIONING_PROFILE_SPECIFIER = '"$(FUTURE_APP_PROFILE)"';
    }
  }
  return project;
}

function withIosShareExtension(config) {
  // Explicit opt-in until Apple App Groups and both signing profiles are ready.
  // Keep existing CI/distribution workflows and their host-only profile intact.
  if (process.env.FUTURE_IOS_SHARE_EXTENSION !== "1") return config;
  config = withEntitlementsPlist(config, mod => {
    const groups = mod.modResults["com.apple.security.application-groups"] ?? [];
    mod.modResults["com.apple.security.application-groups"] = [...new Set([...groups, GROUP])];
    return mod;
  });
  config = withInfoPlist(config, mod => {
    mod.modResults.FutureShareAppGroup = GROUP;
    return mod;
  });
  return withXcodeProject(config, mod => {
    configureProject(mod.modResults, {
      platformRoot: mod.modRequest.platformProjectRoot,
      moduleRoot: path.join(mod.modRequest.projectRoot, "modules", "future-share-intent"),
      version: (mod.version ?? "1.0.0").split(/[-+]/)[0],
      buildNumber: mod.ios?.buildNumber ?? "1",
      manualSigning: process.env.FUTURE_IOS_MANUAL_SIGNING === "1",
    });
    return mod;
  });
}

module.exports = withIosShareExtension;
module.exports.configureProject = configureProject;
