#!/usr/bin/env node
/**
 * Guard: a harness run must leave this machine's real session store untouched.
 *
 * Twelve hours of case runs once added 130 empty sessions to the real library, all
 * with a scratch worktree as their cwd — the fixture answers most commands, but
 * anything it does not handle fell through to a live backend. Nobody noticed until
 * someone read the store by hand.
 *
 * So: count the real store, run whatever you were going to run, count again. One
 * extra session fails the check. That is a number, not an opinion.
 *
 * Usage:
 *   node harness/scripts/assert-no-pollution.mjs -- node harness/cli.mjs run --case X
 *   node harness/scripts/assert-no-pollution.mjs --tag perf -- node harness/cli.mjs run --tag perf
 *
 * Exit: 0 unchanged, 1 the real store moved, 2 could not read it.
 */
import { execFileSync, spawnSync } from "node:child_process";
import path from "node:path";
import os from "node:os";

const argv = process.argv.slice(2);
const sep = argv.indexOf("--");
if (sep < 0 || sep === argv.length - 1) {
  console.error("usage: assert-no-pollution.mjs -- <command ...>");
  process.exit(2);
}
const command = argv.slice(sep + 1);

const REAL_DB = path.join(os.homedir(), ".future", "agent", "agent.db");

function count() {
  const out = spawnSync("python3", ["-c", `
import sqlite3, sys
try:
    c = sqlite3.connect("file:" + sys.argv[1] + "?mode=ro", uri=True)
    print(c.execute("select count(*) from sessions").fetchone()[0])
except Exception:
    print("err")
`, REAL_DB], { encoding: "utf8" });
  const n = Number(out.stdout.trim());
  return Number.isFinite(n) ? n : null;
}

const before = count();
if (before === null) {
  console.error(`could not read ${REAL_DB}`);
  process.exit(2);
}

const run = spawnSync(command[0], command.slice(1), { stdio: "inherit" });
const after = count();

if (after === null) {
  console.error(`could not read ${REAL_DB} after the run`);
  process.exit(2);
}

const delta = after - before;
if (delta !== 0) {
  console.error(`\n✗ 真实会话库被改动：${before} → ${after} (${delta > 0 ? "+" : ""}${delta})`);
  console.error("  用例必须跑在隔离的 FUTURE_HOME 里；真实 ~/.future 只能读。");
  process.exit(1);
}
console.log(`\n✓ 真实会话库未变：${before} 条（用例跑在自己的 FUTURE_HOME 里）`);
process.exit(run.status ?? 0);
