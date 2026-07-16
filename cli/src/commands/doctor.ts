/**
 * `future doctor` — environment diagnostic.
 *
 * Checks:
 *   1. Component installation, versions, and agent connectivity
 *   2. Configuration (auth, models, settings)
 *   3. Session stats (disk + agent)
 *   4. Skills status (local installs + remote updates)
 *   5. Login status + provider/model summary
 *
 * --fix handles: login (auth flow), skills (install builtin).
 * Other issues (missing binaries, bad config) require manual resolution.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import { execFile } from "node:child_process";

import { login, loadAuthFile, getFutureAuthEntry } from "./auth.js";
import { fetchSkills, getInstalledSkillIds, installBuiltinSkills } from "./skills.js";
import { getPlatformUrl } from "../utils/platform.js";
import { which } from "../utils/files.js";
import { RunClient } from "../rpc/grpc-client.js";

// ── Types ──────────────────────────────────────────────────────────────────

interface CheckResult {
  name: string;
  status: "ok" | "warn" | "issue";
  lines: string[];
}

// ── Colors ─────────────────────────────────────────────────────────────────

const C = { reset: "\x1b[0m", bold: "\x1b[1m", dim: "\x1b[2m" };
const GREEN = "\x1b[32m";
const YELLOW = "\x1b[33m";
const RED = "\x1b[31m";

function icon(s: CheckResult["status"]): string {
  if (s === "ok") return `${GREEN}[ok]${C.reset}`;
  if (s === "warn") return `${YELLOW}[--]${C.reset}`;
  return `${RED}[!!]${C.reset}`;
}

function colorStatus(s: CheckResult["status"], text: string): string {
  if (s === "ok") return `${GREEN}${text}${C.reset}`;
  if (s === "warn") return `${YELLOW}${text}${C.reset}`;
  return `${RED}${text}${C.reset}`;
}

// ── Constants ──────────────────────────────────────────────────────────────

const AGENT_DIR = path.join(os.homedir(), ".future", "agent");
const AUTH_FILE = path.join(AGENT_DIR, "auth.json");
const MODELS_FILE = path.join(AGENT_DIR, "models.json");
const SETTINGS_FILE = path.join(AGENT_DIR, "settings.json");
const SESSIONS_DIR = path.join(AGENT_DIR, "sessions");
const GRPC_ADDR = process.env.FUTURE_AGENT_GRPC_ADDR ?? "127.0.0.1:50051";

const SKILL_DIRS: Record<string, string> = {
  app: path.join(os.homedir(), ".future", "agent", "skills"),
  project: path.join(process.cwd(), ".future", "agent", "skills"),
  agents: path.join(os.homedir(), ".agents", "skills"),
};

// ── Entry ──────────────────────────────────────────────────────────────────

export async function doctor(fix: boolean): Promise<void> {
  console.log(`${C.bold}Future Doctor${C.reset} — checking environment...\n`);

  const results: CheckResult[] = [];

  // 1. Component installation & connectivity
  results.push(await checkComponent("future-agent", "Agent"));
  results.push(await checkComponent("future-tui", "TUI"));
  results.push(await checkComponent("future-gui", "GUI"));
  results.push(await checkComponent("future-channel", "Channel bridge"));
  results.push(await checkAgentConnection());

  // 2. Configuration
  results.push(checkAuthConfig());
  results.push(checkModelsConfig());
  results.push(checkSettingsConfig());

  // 3. Session stats
  results.push(await checkSessions());

  // 4. Skills
  results.push(await checkSkills());

  // 5. Login + provider/model summary
  results.push(await checkLogin());
  results.push(await checkProviders());

  printResults(results);

  const issues = results.filter((r) => r.status === "issue");
  const warns = results.filter((r) => r.status === "warn");
  const problemCount = issues.length + warns.length;

  if (problemCount === 0) {
    console.log(`${GREEN}All checks passed.${C.reset}\n`);
    return;
  }

  if (!fix) {
    console.log(
      `${problemCount} issue(s) found. Run \`${C.bold}future doctor --fix${C.reset}\` to attempt automatic repair.\n`,
    );
    return;
  }

  // --fix mode
  console.log(`${C.bold}Attempting repairs...${C.reset}\n`);
  let loginOk = false;
  for (const result of results) {
    if (result.status === "ok") continue;
    if (result.name === "Login") {
      loginOk = await tryFix("Starting login flow...", async () => {
        await login();
      });
    } else if (result.name === "Skills") {
      if (!loginOk) {
        console.log(`  ${YELLOW}Skipping skills install — not logged in.${C.reset}\n`);
      } else {
        await tryFix("Installing builtin skills...", async () => {
          await installBuiltinSkills("app");
        });
      }
    }
    // Other issues (missing binaries, bad config, unreachable agent)
    // are not automatically fixable — the user must resolve them manually.
  }
  console.log(`${C.bold}Done.${C.reset}\n`);
}

// ── 1. Component checks ───────────────────────────────────────────────────

async function checkComponent(bin: string, label: string): Promise<CheckResult> {
  const binPath = await which(bin);
  if (!binPath) {
    return {
      name: label,
      status: "warn",
      lines: [`${bin} not found on PATH — run \`make install\``],
    };
  }
  const version = await getBinaryVersion(binPath);
  return {
    name: label,
    status: "ok",
    lines: version ? [`${binPath}  ${C.dim}(${version})${C.reset}`] : [binPath],
  };
}

/** Run --version and capture only the first meaningful output line (skip log lines). */
function getBinaryVersion(binPath: string): Promise<string | null> {
  return new Promise((resolve) => {
    execFile(binPath, ["--version"], { timeout: 5000 }, (_err, stdout, stderr) => {
      // Some binaries emit logs to stdout alongside the version string.
      // Pick the first line that looks like a version (starts with the binary
      // name or contains a version number), falling back to stderr.
      const candidates = [...stdout.trim().split("\n"), ...stderr.trim().split("\n")];
      const versionLine = candidates.find(
        (line) =>
          !/^\d{4}-\d{2}-\d{2}T/.test(line) &&       // skip ISO timestamp logs
          !/\b(INFO|WARN|ERROR|DEBUG|TRACE)\b/.test(line) && // skip log-level lines
          line.length > 0,
      );
      resolve(versionLine || null);
    });
  });
}

async function checkAgentConnection(): Promise<CheckResult> {
  try {
    const client = new RunClient(GRPC_ADDR);
    const state = await client.getState();
    return {
      name: "Agent connection",
      status: "ok",
      lines: [
        `Connected to ${GRPC_ADDR}`,
        `Version: ${state.version ?? "unknown"}`,
        `Model: ${state.model ?? "none"}`,
        `Thinking: ${state.thinkingLevel ?? "off"}`,
      ],
    };
  } catch {
    return {
      name: "Agent connection",
      status: "issue",
      lines: [`Cannot reach agent at ${GRPC_ADDR} — start with: ${C.bold}future-agent${C.reset}`],
    };
  }
}

// ── 2. Config checks ──────────────────────────────────────────────────────

function checkAuthConfig(): CheckResult {
  if (!fs.existsSync(AUTH_FILE)) {
    return {
      name: "Auth config",
      status: "warn",
      lines: [`${AUTH_FILE} not found — run \`future auth login\` or create manually`],
    };
  }
  try {
    const raw = JSON.parse(fs.readFileSync(AUTH_FILE, "utf-8")) as Record<string, unknown>;
    const keys = Object.keys(raw).filter((k) => {
      const v = raw[k];
      return v && typeof v === "object" && "key" in (v as Record<string, unknown>);
    });
    return {
      name: "Auth config",
      status: keys.length > 0 ? "ok" : "warn",
      lines:
        keys.length > 0
          ? [`${AUTH_FILE} — ${keys.length} provider key(s)`]
          : [`${AUTH_FILE} exists but no keys configured`],
    };
  } catch {
    return {
      name: "Auth config",
      status: "issue",
      lines: [`${AUTH_FILE} exists but is not valid JSON`],
    };
  }
}

function checkModelsConfig(): CheckResult {
  if (!fs.existsSync(MODELS_FILE)) {
    return {
      name: "Models config",
      status: "ok",
      lines: [`${MODELS_FILE} not found (using built-in catalog)`],
    };
  }
  try {
    const raw = JSON.parse(fs.readFileSync(MODELS_FILE, "utf-8")) as Record<string, unknown>;
    const providers = (raw.providers as Record<string, unknown>) ?? {};
    const customIds = Object.keys(providers).filter(
      (id) => id !== "future" && !isOverrideOnly(providers[id]),
    );
    return {
      name: "Models config",
      status: "ok",
      lines: [
        `${MODELS_FILE} exists`,
        customIds.length > 0
          ? `Custom providers: ${customIds.join(", ")}`
          : "No custom providers defined",
      ],
    };
  } catch {
    return {
      name: "Models config",
      status: "issue",
      lines: [`${MODELS_FILE} exists but is not valid JSON`],
    };
  }
}

function isOverrideOnly(config: unknown): boolean {
  if (typeof config !== "object" || config === null) return false;
  const c = config as Record<string, unknown>;
  return !c.name && !c.api && !(Array.isArray(c.models) && c.models.length > 0);
}

function checkSettingsConfig(): CheckResult {
  if (!fs.existsSync(SETTINGS_FILE)) {
    return {
      name: "Agent settings",
      status: "ok",
      lines: [`${SETTINGS_FILE} not found (defaults apply)`],
    };
  }
  try {
    const raw = JSON.parse(fs.readFileSync(SETTINGS_FILE, "utf-8")) as Record<string, unknown>;
    const keys = Object.keys(raw);
    return {
      name: "Agent settings",
      status: "ok",
      lines: [`${SETTINGS_FILE} — ${keys.length} setting(s)`],
    };
  } catch {
    return {
      name: "Agent settings",
      status: "issue",
      lines: [`${SETTINGS_FILE} exists but is not valid JSON`],
    };
  }
}

// ── 3. Session stats ──────────────────────────────────────────────────────

async function checkSessions(): Promise<CheckResult> {
  const lines: string[] = [];
  let jsonlCount = 0;

  if (fs.existsSync(SESSIONS_DIR)) {
    try {
      jsonlCount = fs.readdirSync(SESSIONS_DIR).filter((f) => f.endsWith(".jsonl")).length;
      lines.push(`${jsonlCount} JSONL file(s) in ${SESSIONS_DIR}`);
    } catch {
      lines.push(`Cannot read ${SESSIONS_DIR}`);
    }
  } else {
    lines.push("No session directory — no sessions created yet");
  }

  try {
    const client = new RunClient(GRPC_ADDR);
    const { sessions } = await client.listSessions();
    lines.push(`${sessions.length} session(s) tracked by agent`);
    const recent = sessions
      .sort(
        (a, b) =>
          new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
      )
      .slice(0, 5);
    if (recent.length > 0) {
      lines.push("Recent:");
      for (const s of recent) {
        const label = s.name || s.id.slice(0, 8);
        const model = s.model ? `  [${s.model}]` : "";
        lines.push(`  ${label}${model}`);
      }
    }
  } catch {
    // agent not running — disk stats suffice
  }

  return { name: "Sessions", status: "ok", lines };
}

// ── 4. Skills check ───────────────────────────────────────────────────────

async function checkSkills(): Promise<CheckResult> {
  const lines: string[] = [];
  let localCount = 0;

  for (const [scope, dir] of Object.entries(SKILL_DIRS)) {
    if (!fs.existsSync(dir)) continue;
    try {
      const entries = fs.readdirSync(dir, { withFileTypes: true });
      const skillDirs = entries.filter((e) => {
        if (!e.isDirectory()) return false;
        return fs.existsSync(path.join(dir, e.name, "SKILL.md"));
      });
      if (skillDirs.length > 0) {
        localCount += skillDirs.length;
        lines.push(`  ${scope} (${dir}): ${skillDirs.map((d) => d.name).join(", ")}`);
      }
    } catch {
      // skip unreadable dirs
    }
  }

  if (localCount === 0) {
    lines.push(`No skills installed. Search paths:`);
    for (const [, dir] of Object.entries(SKILL_DIRS)) {
      const exists = fs.existsSync(dir) ? "" : " (missing)";
      lines.push(`  ${dir}${exists}`);
    }
  }

  // Check remote catalog for updates
  try {
    const platformUrl = await getPlatformUrl();
    const builtinSkills = await fetchSkills(platformUrl, "builtin");
    if (builtinSkills.length > 0) {
      lines.push(`${builtinSkills.length} builtin skill(s) available from platform`);
      const installed = await getInstalledSkillIds("app");
      let notInstalled = 0;
      let stale = 0;
      for (const skill of builtinSkills) {
        if (installed.has(skill.id)) {
          const localDir = path.join(SKILL_DIRS.app, skill.id);
          try {
            const md = fs.readFileSync(path.join(localDir, "SKILL.md"), "utf-8");
            const verMatch = md.match(/^version:\s*(.+)$/m);
            if (verMatch && skill.latest_version && verMatch[1].trim() !== skill.latest_version) {
              lines.push(
                `  ${skill.id}: local ${verMatch[1].trim()} ${C.dim}→${C.reset} remote ${skill.latest_version}`,
              );
              stale++;
            }
          } catch {
            // can't read version — skip
          }
        } else {
          notInstalled++;
        }
      }
      if (notInstalled > 0) lines.push(`  ${notInstalled} not installed`);
      if (stale > 0) lines.push(`  ${stale} have updates available`);
    }
  } catch {
    // offline or not logged in
  }

  return {
    name: "Skills",
    status: localCount > 0 ? "ok" : "warn",
    lines,
  };
}

// ── 5. Login + providers ──────────────────────────────────────────────────

async function checkLogin(): Promise<CheckResult> {
  try {
    const auth = await loadAuthFile();
    const entry = getFutureAuthEntry(auth);
    if (entry?.key) {
      const platformUrl = entry.base_url
        ? entry.base_url.replace(/\/api\/?$/, "")
        : await getPlatformUrl().catch(() => "unknown");
      return {
        name: "Login",
        status: "ok",
        lines: [`Logged in to ${platformUrl}`],
      };
    }
  } catch {
    // fall through
  }
  return {
    name: "Login",
    status: "warn",
    lines: ["Not logged in — run `future auth login`"],
  };
}

async function checkProviders(): Promise<CheckResult> {
  const lines: string[] = [];
  let keyCount = 0;
  let providerCount = 0;

  // Collect providers with keys
  const keyedProviders: string[] = [];
  try {
    if (fs.existsSync(AUTH_FILE)) {
      const raw = JSON.parse(fs.readFileSync(AUTH_FILE, "utf-8")) as Record<string, unknown>;
      for (const [id, v] of Object.entries(raw)) {
        if (v && typeof v === "object" && "key" in (v as Record<string, unknown>)) {
          keyCount++;
          if (id !== "future") keyedProviders.push(id);
        }
      }
    }
  } catch {
    // ignore
  }

  // Count custom providers from models.json
  try {
    if (fs.existsSync(MODELS_FILE)) {
      const raw = JSON.parse(fs.readFileSync(MODELS_FILE, "utf-8")) as Record<string, unknown>;
      const providers = (raw.providers as Record<string, unknown>) ?? {};
      const entries = Object.entries(providers).filter(([id]) => id !== "future");
      providerCount = entries.length;
      for (const [id, config] of entries) {
        if (isOverrideOnly(config)) continue;
        const c = config as Record<string, unknown>;
        const models = Array.isArray(c.models) ? c.models : [];
        const hasKey = keyedProviders.includes(id);
        const keyNote = hasKey ? " [key]" : "";
        lines.push(`  ${id}: ${models.length} model(s)${keyNote}`);
      }
    }
  } catch {
    // ignore
  }

  // If agent is reachable, ask it for built-in model count
  try {
    const client = new RunClient(GRPC_ADDR);
    const state = await client.getState();
    if (state.model) {
      lines.push(`  Current model: ${state.model}`);
    }
    if (state.contextWindow) {
      lines.push(`  Context window: ${state.contextWindow.toLocaleString()}`);
    }
  } catch {
    // agent not running
  }

  lines.unshift(`${keyCount} provider(s) with API key, ${providerCount} in models.json`);

  return {
    name: "Providers & models",
    status: "ok",
    lines,
  };
}

// ── Output ─────────────────────────────────────────────────────────────────

function printResults(results: CheckResult[]): void {
  for (const r of results) {
    const label = colorStatus(r.status, r.name);
    console.log(`${icon(r.status)} ${label}`);
    for (const line of r.lines) {
      console.log(`      ${line}`);
    }
    console.log();
  }
}

async function tryFix(label: string, fn: () => Promise<void>): Promise<boolean> {
  console.log(`  ${C.dim}Fixing:${C.reset} ${label}`);
  try {
    await fn();
    console.log(`  ${GREEN}Done.${C.reset}\n`);
    return true;
  } catch (err) {
    console.log(`  ${RED}Failed:${C.reset} ${err instanceof Error ? err.message : String(err)}\n`);
    return false;
  }
}
