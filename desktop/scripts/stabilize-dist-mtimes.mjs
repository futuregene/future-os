// Keep `dist/` mtimes stable so an unchanged frontend does not rebuild the
// Rust crate.
//
// Vite rewrites every file under `dist/` on each build, so byte-identical
// outputs come out with brand-new mtimes. Tauri embeds `dist/` into the crate
// (`tauri::generate_context!` expands to `include_bytes!` for every asset), and
// cargo rebuilds that crate whenever an embedded file's mtime changes — so
// every `make install` / `tauri build` paid a full ~2min desktop rebuild even
// with no frontend change and no source change at all.
//
// Run right after `vite build`: for every output whose bytes are unchanged,
// write back the timestamp recorded the last time, so cargo sees an untouched
// frontend. Only timestamps are ever rewritten, never content, so a stale
// `dist/` can never be embedded — a changed asset keeps its fresh mtime and the
// crate rebuilds as it must.
//
// The state file records the timestamp actually on disk *after* the write,
// because that is what cargo stores in its fingerprint. Writing a value through
// the seconds-as-double conversion in `utimes` is not bit-exact but it is a
// fixed point, so re-applying a recorded value is a no-op for cargo.
import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, statSync, utimesSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const desktopDir = fileURLToPath(new URL("..", import.meta.url));
const distDir = join(desktopDir, "dist");
const statePath = join(desktopDir, "node_modules", ".cache", "futureos-dist-mtimes.json");

/** Every file under `dir`, recursively. */
function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    return entry.isDirectory() ? walk(path) : [path];
  });
}

/** `{ [distRelativePath]: { hash, mtimeNs } }`, or `{}` when absent/unreadable. */
function readState() {
  try {
    return JSON.parse(readFileSync(statePath, "utf8"));
  }
  catch {
    return {};
  }
}

/** Set the file's mtime and return the value the filesystem ended up storing. */
function applyMtime(path, mtimeNs) {
  const { atimeNs } = statSync(path, { bigint: true });
  utimesSync(path, Number(atimeNs) / 1e9, Number(mtimeNs) / 1e9);
  return statSync(path, { bigint: true }).mtimeNs;
}

function main() {
  let files;
  try {
    files = walk(distDir);
  }
  catch (error) {
    console.warn(`frontend dist mtimes: skipped (${error.message})`);
    return;
  }

  const previous = readState();
  const current = {};
  let preserved = 0;
  let changed = 0;

  for (const path of files) {
    const name = relative(distDir, path).split("\\").join("/");
    const hash = createHash("sha256").update(readFileSync(path)).digest("hex");
    const recorded = previous[name];
    // Unchanged content keeps the timestamp cargo already has for it; anything
    // else is normalized to a value we can reproduce verbatim next time.
    const mtimeNs = recorded?.hash === hash
      ? recorded.mtimeNs
      : statSync(path, { bigint: true }).mtimeNs;
    current[name] = { hash, mtimeNs: applyMtime(path, mtimeNs).toString() };
    if (recorded?.hash === hash) {
      preserved++;
    }
    else {
      changed++;
    }
  }

  mkdirSync(dirname(statePath), { recursive: true });
  writeFileSync(statePath, `${JSON.stringify(current)}\n`);
  console.log(`frontend dist: ${preserved} unchanged (mtime preserved), ${changed} changed/new`);
}

main();
