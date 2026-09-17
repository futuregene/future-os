const path = require("node:path");
const { getDefaultConfig } = require("expo/metro-config");

const config = getDefaultConfig(__dirname);

// The NATS key libraries contain guarded Node fallbacks. Metro still resolves
// those requires even though React Native supplies these globals at runtime.
config.resolver.extraNodeModules = {
  ...config.resolver.extraNodeModules,
  crypto: path.resolve(__dirname, "src/polyfills/crypto.ts"),
  util: path.resolve(__dirname, "src/polyfills/util.ts"),
  "sodium-universal": require.resolve("sodium-javascript"),
  "sodium-native": require.resolve("sodium-javascript"),
};

// MathJax uses a package-import alias for its default font. Metro does not
// resolve that external "imports" target yet. Use the same self-contained TeX
// SVG font as our renderer rather than bundling the dynamic browser font.
const resolveDefault = (context, moduleName, platform) => context.resolveRequest(
  context,
  moduleName === "#default-font/svg/default.js"
    ? "@mathjax/mathjax-tex-font/mjs/svg/default.js"
    : moduleName,
  platform,
);

/**
 * Screenshot harness (`make screenshots-mobile`, see docs/guide/screenshots.md).
 *
 * On the web platform only, and only when SHOT_WEB=1, the NATS-backed remote
 * context and the two native modules it depends on resolve to the stand-ins
 * under `shot/mock/`. Everything else — screens, components, projection, i18n —
 * stays the shipped implementation, so the Expo web build renders the real UI
 * against a fixed dataset. Native builds never take this path.
 */
const WEB_SHOT_MOCKS = [
  [/remote[/\\]RemoteContext$/, "shot/mock/RemoteContext.tsx"],
  [/^future-share-intent$/, "shot/mock/shareIntent.ts"],
  [/^future-file-handler$/, "shot/mock/fileHandler.ts"],
];

/**
 * Force one React for the web bundle. This npm workspace hoists packages to the
 * repo root, so `react` (imported by app code under `mobile/`) and `react-dom`
 * (imported by hoisted web dependencies such as expo's metro runtime) can
 * resolve to two different copies; React then throws "Incompatible React
 * versions" and the page stays blank.
 *
 * `extraNodeModules` is only a *fallback* for unresolvable names, and
 * `react-dom` resolves fine from the hoisted root, so the override has to happen
 * here — and it has to yield the module's entry file, not its directory (Metro
 * would otherwise load `package.json` as the module).
 */
function pinnedReactEntry(moduleName) {
  if (process.env.SHOT_WEB !== "1")
    return null;
  // Exact names and their subpaths (`react-dom/client`, `react/jsx-runtime`),
  // but not lookalikes such as react-native, react-i18next or react-is.
  const isReact = moduleName === "react" || moduleName.startsWith("react/");
  const isReactDom = moduleName === "react-dom" || moduleName.startsWith("react-dom/");
  if (!isReact && !isReactDom)
    return null;
  try {
    // Resolve as the app would, so `mobile/node_modules` wins over the root.
    return require.resolve(moduleName, { paths: [__dirname] });
  }
  catch {
    return null;
  }
}

config.resolver.resolveRequest = (context, moduleName, platform) => {
  if (platform === "web") {
    const pinned = pinnedReactEntry(moduleName);
    if (pinned)
      return { type: "sourceFile", filePath: pinned };
    if (process.env.SHOT_WEB === "1") {
      const hit = WEB_SHOT_MOCKS.find(([pattern]) => pattern.test(moduleName));
      if (hit)
        return { type: "sourceFile", filePath: path.resolve(__dirname, hit[1]) };
    }
  }
  return resolveDefault(context, moduleName, platform);
};

module.exports = config;
