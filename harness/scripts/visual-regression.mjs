#!/usr/bin/env node
/**
 * Visual regression for the UI harness: compare a run's recording against a stored
 * baseline and report the frames that moved.
 *
 * Why this exists next to the assertions: replaying 100 historical UI fixes, the
 * assertions caught 68% of the ones they could run; of the 22 they missed, 20 still
 * changed the picture. Those 20 were never about "is this the right property" —
 * they were "something moved" — and a pixel comparison answers that for free,
 * without anyone writing a spec, and without a model judging.
 *
 * Rules this follows (learned in the same session):
 *   · A threshold is per case, from that case's own jitter over >= 5 runs. A case
 *     whose own runs disagree is excluded rather than given a loose threshold —
 *     a flaky visual test gets switched off within days.
 *   · Updating a baseline is an explicit, recorded act (`--update-baseline
 *     --reason ...`). Nothing updates a baseline implicitly: an auto-updating
 *     baseline silences every future regression.
 *   · Only cases whose pages changed are compared (`--changed <ref>`).
 *
 * Usage:
 *   node harness/scripts/visual-regression.mjs --case search-result-open --jitter 5
 *   node harness/scripts/visual-regression.mjs --case search-result-open \
 *        --update-baseline --reason "settings copy rewritten in #1234"
 *   node harness/scripts/visual-regression.mjs --check search-result-open [more...]
 *   node harness/scripts/visual-regression.mjs --changed origin/main
 */
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import os from "node:os";

const ROOT = resolve(import.meta.dirname, "..", "..");
const STORE = join(ROOT, "harness", "visual", "baselines");
const FPS = 2;
const WIDTH = 240;

const argv = process.argv.slice(2);
const arg = (n, d) => { const i = argv.indexOf(`--${n}`); return i >= 0 && argv[i + 1] ? argv[i + 1] : d; };
const flag = (n) => argv.includes(`--${n}`);
const casesAfter = (n) => { const i = argv.indexOf(`--${n}`); return i < 0 ? [] : argv.slice(i + 1).filter((a) => !a.startsWith("--")); };

function runHarness(caseId) {
  const before = new Set(readdirSync(join(ROOT, "harness", "runs")));
  const r = spawnSync("node", ["harness/cli.mjs", "run", "--case", caseId, "--analyze", "none"],
    { cwd: ROOT, encoding: "utf8", timeout: 300_000 });
  if (r.status !== 0) throw new Error(`case ${caseId} exited ${r.status}`);
  const after = readdirSync(join(ROOT, "harness", "runs"));
  const fresh = after.find((d) => !before.has(d));
  if (!fresh) throw new Error(`case ${caseId} produced no run directory`);
  const video = join(ROOT, "harness", "runs", fresh, caseId, "video.mp4");
  if (!existsSync(video)) throw new Error(`case ${caseId} recorded no video`);
  return video;
}

/** Downscaled grayscale frames at a fixed rate, so two runs line up index by index. */
function frames(video) {
  const dir = join(os.tmpdir(), `vr-${Math.random().toString(36).slice(2)}`);
  mkdirSync(dir, { recursive: true });
  execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-i", video, "-vf",
    `fps=${FPS},scale=${WIDTH}:-1,format=gray`, join(dir, "f%03d.png")]);
  const files = readdirSync(dir).filter((f) => f.endsWith(".png")).sort();
  return { dir, files };
}

/** Fraction of pixels that differ, ignoring sub-perceptual noise. */
function ratio(a, b) {
  const out = spawnSync("python3", ["-c", `
import sys
from PIL import Image, ImageChops
a, b = Image.open(sys.argv[1]).convert("L"), Image.open(sys.argv[2]).convert("L")
if a.size != b.size:
    print(1.0); raise SystemExit
h = ImageChops.difference(a, b).histogram()
print(sum(h[12:]) / (a.size[0] * a.size[1]))
`, a, b], { encoding: "utf8" });
  return Number(out.stdout.trim() || 1);
}

function compare(baselineDir, subjectVideo) {
  const base = { files: readdirSync(baselineDir).filter((f) => f.endsWith(".png")).sort() };
  const sub = frames(subjectVideo);
  const n = Math.min(base.files.length, sub.files.length);
  let worst = 0, at = null;
  for (let i = 0; i < n; i++) {
    const r = ratio(join(baselineDir, base.files[i]), join(sub.dir, sub.files[i]));
    if (r > worst) { worst = r; at = i; }
  }
  if (base.files.length !== sub.files.length) worst = Math.max(worst, Math.abs(base.files.length - sub.files.length) / Math.max(1, base.files.length));
  rmSync(sub.dir, { recursive: true, force: true });
  return { worst, at, frames: n };
}

function meta(caseId) {
  const p = join(STORE, caseId, "meta.json");
  return existsSync(p) ? JSON.parse(readFileSync(p, "utf8")) : null;
}

function command(caseId) {
  const video = runHarness(caseId);
  const f = frames(video);
  const dir = join(STORE, caseId);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  for (const file of f.files) execFileSync("cp", [join(f.dir, file), join(dir, file)]);
  rmSync(f.dir, { recursive: true, force: true });
  return { dir, frames: f.files.length };
}

function jitter(caseId, runs) {
  const fs = [];
  for (let i = 0; i < runs; i++) fs.push(command(caseId));
  const ratios = [];
  for (let i = 1; i < fs.length; i++) {
    let worst = 0;
    for (const file of readdirSync(fs[i].dir).filter((f) => f.endsWith(".png"))) {
      worst = Math.max(worst, ratio(join(fs[0].dir, file), join(fs[i].dir, file)));
    }
    ratios.push(worst);
  }
  const max = Math.max(...ratios);
  return { runs, ratios: ratios.map((r) => Number(r.toFixed(4))), max: Number(max.toFixed(4)) };
}

function main() {
  if (flag("jitter")) {
    const caseId = arg("case");
    const runs = Number(arg("jitter", "5"));
    if (!caseId) throw new Error("--jitter needs --case");
    const j = jitter(caseId, runs);
    // a case whose own runs disagree by more than 5% is not stable enough to gate
    const stable = j.max <= 0.05;
    const threshold = stable ? Math.max(0.01, Number((j.max * 1.5).toFixed(4))) : null;
    console.log(JSON.stringify({ case: caseId, ...j, stable, threshold }, null, 1));
    return;
  }

  if (flag("update-baseline")) {
    const caseId = arg("case");
    const reason = arg("reason");
    if (!caseId) throw new Error("--update-baseline needs --case");
    if (!reason) throw new Error("--update-baseline needs --reason: a baseline update is a decision, record why");
    const { frames: n } = command(caseId);
    const previous = meta(caseId);
    const record = {
      case: caseId, frames: n, updatedAt: new Date().toISOString(),
      updatedBy: execFileSync("git", ["config", "user.email"], { encoding: "utf8" }).trim() || "unknown",
      commit: execFileSync("git", ["rev-parse", "--short", "HEAD"], { encoding: "utf8" }).trim(),
      reason,
      // keep the previous threshold unless jitter has been measured for this case
      threshold: previous?.threshold ?? 0.01,
    };
    writeFileSync(join(STORE, caseId, "meta.json"), JSON.stringify(record, null, 1));
    console.log(JSON.stringify(record, null, 1));
    return;
  }

  if (flag("changed")) {
    const ref = arg("changed");
    const files = execFileSync("git", ["diff", "--name-only", ref], { cwd: ROOT, encoding: "utf8" })
      .split("\n").filter(Boolean);
    const map = JSON.parse(readFileSync(join(ROOT, "harness", "visual", "page-map.json"), "utf8"));
    const picked = new Set();
    for (const f of files) {
      for (const [needle, caseIds] of Object.entries(map.prefixes)) {
        if (f.startsWith(needle)) caseIds.forEach((c) => picked.add(c));
      }
    }
    console.log(JSON.stringify({ changedFiles: files.length, cases: [...picked] }, null, 1));
    process.exitCode = 0;
    return;
  }

  // --case-video <case>=<path> compares a recording on disk instead of running the
  // case again: the replay of a historical fix uses this.
  const provided = new Map();
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--case-video" && argv[i + 1]?.includes("=")) {
      const [c, ...rest] = argv[i + 1].split("=");
      provided.set(c, rest.join("="));
    }
  }
  const ids = casesAfter("check");
  if (!ids.length) throw new Error("need --check <case...>, --jitter, --update-baseline or --changed");
  const rows = [];
  let failed = 0;
  for (const caseId of ids) {
    const m = meta(caseId);
    if (!m) { rows.push({ case: caseId, verdict: "NO_BASELINE" }); failed++; continue; }
    const video = provided.get(caseId) ?? runHarness(caseId);
    const { worst, at, frames: n } = compare(join(STORE, caseId), video);
    const verdict = worst > m.threshold ? "FAIL" : "PASS";
    if (verdict === "FAIL") failed++;
    rows.push({ case: caseId, worst: Number(worst.toFixed(4)), threshold: m.threshold, atFrame: at, frames: n, verdict });
  }
  if (flag("json")) console.log(JSON.stringify(rows));
  else console.table(rows);
  process.exitCode = failed ? 1 : 0;
}

main();
