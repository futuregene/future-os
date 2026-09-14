// Run after Expo prebuild. Tests actual generated Xcode/entitlement wiring, not
// a hand-written pbxproj fixture. Temporary repeat-prebuild output is removed.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const xcode = require("xcode");
const plist = require("@expo/plist").default;
const withIosShareExtension = require("../mobile/plugins/withIosShareExtension");
const { configureProject } = withIosShareExtension;
const originalOptIn = process.env.FUTURE_IOS_SHARE_EXTENSION;
try {
  delete process.env.FUTURE_IOS_SHARE_EXTENSION;
  const config = { name: "FutureOS", slug: "futureos" };
  assert.equal(withIosShareExtension(config), config, "existing builds must not gain extension entitlements");
  process.env.FUTURE_IOS_SHARE_EXTENSION = "1";
  assert.ok(withIosShareExtension({ ...config }).mods.ios.xcodeproj, "opt-in must register native project generation");
} finally {
  if (originalOptIn === undefined) delete process.env.FUTURE_IOS_SHARE_EXTENSION;
  else process.env.FUTURE_IOS_SHARE_EXTENSION = originalOptIn;
}
const root = path.resolve(__dirname, "..");
const ios = path.join(root, "mobile/ios");
const readPlist = file => plist.parse(fs.readFileSync(file, "utf8"));
const group = "group.cn.futureos.mobile.share";
const host = readPlist(path.join(ios, "FutureOS/Info.plist"));
const hostEntitlements = readPlist(path.join(ios, "FutureOS/FutureOS.entitlements"));
assert.equal(host.FutureShareAppGroup, group);
assert.ok(hostEntitlements["com.apple.security.application-groups"].includes(group));
const project = xcode.project(path.join(ios, "FutureOS.xcodeproj/project.pbxproj"));
project.parseSync();
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "future-ios-project-"));
try {
  for (const buildNumber of ["41", "42"]) {
    configureProject(project, {
      platformRoot: temporary,
      moduleRoot: path.join(root, "mobile/modules/future-share-intent"),
      version: "1.2.3", buildNumber, manualSigning: true,
    });
  }
  const targets = Object.entries(project.pbxNativeTargetSection()).filter(([, target]) =>
    typeof target === "object" && target.name.replaceAll('"', "") === "FutureShareExtension");
  assert.equal(targets.length, 1, "repeat prebuild must not duplicate the extension");
  const [id, extension] = targets[0];
  const app = project.getFirstTarget().firstTarget;
  assert.ok(app.dependencies.some(ref => project.hash.project.objects.PBXTargetDependency[ref.value].target === id));
  const copyPhases = app.buildPhases.map(ref => project.hash.project.objects.PBXCopyFilesBuildPhase?.[ref.value]).filter(Boolean);
  assert.ok(copyPhases.some(phase => phase.dstSubfolderSpec === 13 && phase.files.some(ref =>
    project.hash.project.objects.PBXBuildFile[ref.value].fileRef === extension.productReference)));
  for (const ref of project.pbxXCConfigurationList()[extension.buildConfigurationList].buildConfigurations) {
    const settings = project.pbxXCBuildConfigurationSection()[ref.value].buildSettings;
    assert.equal(settings.CURRENT_PROJECT_VERSION, '"42"');
    assert.equal(settings.MARKETING_VERSION, '"1.2.3"');
    assert.equal(settings.PROVISIONING_PROFILE_SPECIFIER, '"$(FUTURE_SHARE_PROFILE)"');
    assert.equal(settings.APPLICATION_EXTENSION_API_ONLY, "YES");
  }
  for (const ref of project.pbxXCConfigurationList()[app.buildConfigurationList].buildConfigurations) {
    assert.equal(project.pbxXCBuildConfigurationSection()[ref.value].buildSettings.PROVISIONING_PROFILE_SPECIFIER, '"$(FUTURE_APP_PROFILE)"');
  }
  const extRoot = path.join(temporary, "FutureShareExtension");
  const extPlist = readPlist(path.join(extRoot, "FutureShareExtension-Info.plist"));
  assert.equal(extPlist.FutureShareAppGroup, group);
  assert.equal(extPlist.NSExtension.NSExtensionPointIdentifier, "com.apple.share-services");
  assert.equal(extPlist.NSExtension.NSExtensionAttributes.NSExtensionActivationRule.NSExtensionActivationSupportsImageWithMaxCount, 10);
  assert.deepEqual(readPlist(path.join(extRoot, "FutureShareExtension.entitlements"))["com.apple.security.application-groups"], [group]);
  const sources = extension.buildPhases.map(ref => project.hash.project.objects.PBXSourcesBuildPhase?.[ref.value]).filter(Boolean);
  assert.equal(sources.length, 1);
  assert.equal(sources[0].files.length, 2);
  for (const file of ["ShareInbox.swift", "ShareViewController.swift"]) assert.ok(fs.existsSync(path.join(extRoot, file)));
  console.log("iOS project: extension embedding, shared entitlements, source membership, repeat prebuild, versions and per-target signing passed");
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
