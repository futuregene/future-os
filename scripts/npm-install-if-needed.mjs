// Install the workspace npm deps only when the manifests moved on.
//
// The shared packages, desktop, and mobile are npm workspaces, so one
// `npm install` at the repo root installs all of them. The Makefile used to
// branch per platform here: POSIX compared manifest mtimes against npm's
// install stamp, while Windows ran `npm install` on every build because
// cmd/make has no usable mtime test — measured at 6-13s of a ~13s
// `make install`. Node is already required by the Makefile
// (scripts/version.mjs), so the check lives here and both platforms share it.
//
// npm writes `node_modules/.package-lock.json` on every install; anything
// newer than that stamp means the tree moved on and the deps must be refreshed.
// A missing stamp (fresh clone, deleted node_modules) always reinstalls.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const stamp = join(root, "node_modules", ".package-lock.json");

/** Every manifest npm reads for this tree: the root, the lockfile, each workspace. */
function manifests() {
  const paths = ["package.json", "package-lock.json"];
  // npm tolerates a UTF-8 BOM in a manifest (Windows editors add them) but
  // JSON.parse does not, so strip it before parsing.
  const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8").replace(/^\uFEFF/, ""));
  for (const entry of manifest.workspaces ?? []) {
    if (!entry.includes("*")) {
      paths.push(join(entry, "package.json"));
      continue;
    }
    const [prefix, suffix] = entry.split("*");
    for (const child of readdirSync(join(root, prefix), { withFileTypes: true })) {
      if (child.isDirectory()) {
        paths.push(join(prefix, child.name, suffix, "package.json"));
      }
    }
  }
  return paths.filter(path => existsSync(join(root, path)));
}

const installedAt = existsSync(stamp) ? statSync(stamp).mtimeMs : Number.NaN;
const stale = !existsSync(stamp)
  || manifests().some(path => statSync(join(root, path)).mtimeMs > installedAt);

if (stale) {
  console.log("  npm install (workspace manifests changed)");
  // npm is npm.cmd on Windows, which cannot be spawned directly (Node rejects
  // .cmd/.bat without a shell) — but `shell: true` cannot be combined with
  // arguments without a deprecation warning, so go through cmd.exe explicitly.
  // The command string is fixed, so no argument needs escaping.
  const [command, args] = process.platform === "win32"
    ? [process.env.ComSpec ?? "cmd.exe", ["/d", "/s", "/c", "npm install --silent"]]
    : ["npm", ["install", "--silent"]];
  try {
    execFileSync(command, args, { cwd: root, stdio: "inherit" });
  }
  catch (error) {
    console.error(`  npm install failed: ${error.message}`);
    process.exit(error.status ?? 1);
  }
}
