/**
 * Bundle the phone's remote client for Node.
 *
 * `esbuild` with three aliases is the whole trick: the app's own pairing and
 * client modules are compiled and bundled as they are, and only the platform
 * packages (`expo-device`, `expo-secure-store`, `react-native`) point at the
 * shims next to this file. Bundling rather than `tsx` keeps the import graph
 * exactly what the device runs — including the workspace packages the phone
 * shares with the desktop.
 */
import { build } from "esbuild";
import { mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const here = dirname(new URL(import.meta.url).pathname);
const outfile = join(here, ".artifacts", "mobile-client.cjs");
mkdirSync(dirname(outfile), { recursive: true });

await build({
  entryPoints: [join(here, "entry.mjs")],
  outfile,
  bundle: true,
  platform: "node",
  format: "cjs",
  target: "node24",
  sourcemap: "inline",
  // The workspace packages ship `.ts` sources; esbuild compiles them as part of
  // the bundle, which is why this needs no separate build step for them.
  loader: { ".ts": "ts", ".tsx": "tsx" },
  alias: {
    "expo-device": join(here, "shims", "expo-device.mjs"),
    "expo-secure-store": join(here, "shims", "expo-secure-store.mjs"),
    "react-native": join(here, "shims", "react-native.mjs"),
  },
  // Native addons cannot be bundled: `sodium-native` loads a `.node` build for
  // the running platform, so it has to resolve from node_modules at run time.
  // Everything else, including the workspace packages' TS sources, is inlined.
  external: ["node:*", "sodium-native", "sodium-universal", "node-gyp-build", "nkeys.js"],
  logLevel: "error",
});

console.log(`wrote ${resolve(outfile)}`);
