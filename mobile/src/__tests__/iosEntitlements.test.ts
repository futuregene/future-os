/** @jest-environment node */

import { execFileSync } from "node:child_process";
import path from "node:path";

test("generated iOS entitlements keep keychain access without requesting APNs", () => {
  // Run the real app config and installed Expo plugins together: testing the
  // cleanup callback alone misses an upstream plugin adding APNs back later.
  const expoCli = path.join(path.dirname(require.resolve("expo/package.json")), "bin/cli");
  const output = execFileSync(process.execPath, [expoCli, "config", "--type", "introspect", "--json"], {
    cwd: path.resolve(__dirname, "../.."),
    encoding: "utf8",
    timeout: 30_000,
    maxBuffer: 4 * 1024 * 1024,
  });
  const config = JSON.parse(output);
  const entitlements = config._internal.modResults.ios.entitlements;
  expect(entitlements).not.toHaveProperty("aps-environment");
  expect(entitlements["keychain-access-groups"]).toEqual([
    "$(AppIdentifierPrefix)$(CFBundleIdentifier)",
  ]);
}, 35_000);
