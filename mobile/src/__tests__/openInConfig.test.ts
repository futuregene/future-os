/** @jest-environment node */

import { execFileSync } from "node:child_process";
import path from "node:path";

// Inspect actual plugin output rather than just the handwritten Expo config.
test("native Open In registration covers documents/images without claiming web or pairing links", () => {
  const expoCli = path.join(path.dirname(require.resolve("expo/package.json")), "bin/cli");
  const config = JSON.parse(execFileSync(process.execPath, [expoCli, "config", "--type", "introspect", "--json"], {
    cwd: path.resolve(__dirname, "../.."), encoding: "utf8", timeout: 30_000,
    maxBuffer: 4 * 1024 * 1024,
    env: { ...process.env, FUTURE_IOS_SHARE_EXTENSION: "0" },
  }));
  const plist = config._internal.modResults.ios.infoPlist;
  expect(plist.LSSupportsOpeningDocumentsInPlace).toBe(false);
  // Introspection merges existing generated plist keys. Read public config
  // separately to prove a fresh host-only build needs no extension/App Group.
  const publicConfig = JSON.parse(execFileSync(process.execPath, [expoCli, "config", "--type", "public", "--json"], {
    cwd: path.resolve(__dirname, "../.."), encoding: "utf8", timeout: 10_000,
    env: { ...process.env, FUTURE_IOS_SHARE_EXTENSION: "0" },
  }));
  expect(publicConfig.ios.infoPlist.FutureShareAppGroup).toBeUndefined();
  expect(publicConfig.ios.infoPlist.CFBundleDocumentTypes).toEqual(plist.CFBundleDocumentTypes);
  const types = plist.CFBundleDocumentTypes.flatMap((type: { LSItemContentTypes: string[] }) => type.LSItemContentTypes);
  expect(types).toEqual(expect.arrayContaining([
    "public.image", "public.text", "com.adobe.pdf", "com.microsoft.word.doc",
    "org.openxmlformats.wordprocessingml.document", "com.microsoft.powerpoint.ppt",
    "org.openxmlformats.presentationml.presentation", "net.daringfireball.markdown",
  ]));
  const extensions = plist.UTImportedTypeDeclarations.flatMap(
    (type: { UTTypeTagSpecification: { "public.filename-extension": string[] } }) =>
      type.UTTypeTagSpecification["public.filename-extension"],
  );
  expect(extensions).toEqual(expect.arrayContaining(["doc", "docx", "ppt", "pptx", "md", "xlsx"]));

  const activity = config._internal.modResults.android.manifest.manifest.application[0].activity[0];
  type Filter = { action: { $: { "android:name": string } }[]; data?: { $: Record<string, string> }[] };
  const filters: Filter[] = activity["intent-filter"];
  const view = filters.find(filter => filter.action.some(action => action.$["android:name"] === "android.intent.action.VIEW")
    && filter.data?.some(data => data.$["android:mimeType"]));
  expect(view).toBeDefined();
  expect(view!.data!.every(data => data.$["android:scheme"] === "content")).toBe(true);
  expect(view!.data!.map(data => data.$["android:mimeType"])).toEqual(expect.arrayContaining([
    "application/pdf", "application/msword", "text/*", "image/*",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  ]));
  // Existing sharing still has its own entry points.
  for (const action of ["SEND", "SEND_MULTIPLE"]) {
    expect(filters.some(filter => filter.action.some(item => item.$["android:name"] === `android.intent.action.${action}`))).toBe(true);
  }
}, 35_000);
