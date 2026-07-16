/**
 * `future doctor` — environment diagnostic.
 *
 * Checks:
 *   1. Login status
 *   2. Component installation & versions
 *   3. Agent connectivity
 *   4. Configuration (auth keys, models, settings)
 *   5. Providers & models
 *   6. Sessions
 *   7. Skills
 */

import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
import { execFile } from "node:child_process";

import { loadAuthFile, getFutureAuthEntry } from "./auth.js";
import {
  fetchSkills,
  getInstalledSkillIds,
  readSkillMdVersion,
  SKILLS_DIR,
} from "./skills.js";
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

function colorName(s: CheckResult["status"], text: string): string {
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

// ── Entry ──────────────────────────────────────────────────────────────────

export async function doctor(): Promise<void> {
  console.log(`${C.bold}Future Doctor${C.reset} — checking environment...\n`);

  const results: CheckResult[] = [];

  // 1. Login
  results.push(await checkLogin());

  // 2. Components
  results.push(await checkComponent("future-agent", "Agent"));
  results.push(await checkComponent("future-tui", "TUI"));
  results.push(await checkComponent("future-gui", "GUI"));
  results.push(await checkComponent("future-channel", "Channel bridge"));

  // 3. Agent connectivity
  results.push(await checkAgentConnection());

  // 4. Configuration
  results.push(checkAuthConfig());
  results.push(checkModelsConfig());
  results.push(checkSettingsConfig());

  // 5. Providers & models
  results.push(await checkProviders());

  // 6. Sessions
  results.push(await checkSessions());

  // 7. Skills
  results.push(await checkSkills());

  printResults(results);

  const issues = results.filter((r) => r.status === "issue");
  const warns = results.filter((r) => r.status === "warn");
  const problemCount = issues.length + warns.length;

  if (problemCount === 0) {
    console.log(`${GREEN}All checks passed.${C.reset}\n`);
  }
}

// ── 1. Login ───────────────────────────────────────────────────────────────

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

// ── 2. Components ──────────────────────────────────────────────────────────

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

function getBinaryVersion(binPath: string): Promise<string | null> {
  return new Promise((resolve) => {
    execFile(binPath, ["--version"], { timeout: 5000 }, (_err, stdout, stderr) => {
      const candidates = [...stdout.trim().split("\n"), ...stderr.trim().split("\n")];
      const versionLine = candidates.find(
        (line) =>
          !/^\d{4}-\d{2}-\d{2}T/.test(line) &&
          !/\b(INFO|WARN|ERROR|DEBUG|TRACE)\b/.test(line) &&
          line.length > 0,
      );
      resolve(versionLine || null);
    });
  });
}

// ── 3. Agent connectivity ──────────────────────────────────────────────────

async function checkAgentConnection(): Promise<CheckResult> {
  try {
    const client = new RunClient(GRPC_ADDR);
    const state = await client.getState();
    return {
      name: "Agent",
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
      name: "Agent",
      status: "issue",
      lines: [`Cannot reach agent at ${GRPC_ADDR} — start with: ${C.bold}future-agent${C.reset}`],
    };
  }
}

// ── 4. Configuration ──────────────────────────────────────────────────────

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
    JSON.parse(fs.readFileSync(SETTINGS_FILE, "utf-8")) as Record<string, unknown>;
    return {
      name: "Agent settings",
      status: "ok",
      lines: [`${SETTINGS_FILE} exists`],
    };
  } catch {
    return {
      name: "Agent settings",
      status: "issue",
      lines: [`${SETTINGS_FILE} exists but is not valid JSON`],
    };
  }
}

// ── 5. Providers & models ──────────────────────────────────────────────────

async function checkProviders(): Promise<CheckResult> {
  const lines: string[] = [];
  let keyCount = 0;

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

  try {
    if (fs.existsSync(MODELS_FILE)) {
      const raw = JSON.parse(fs.readFileSync(MODELS_FILE, "utf-8")) as Record<string, unknown>;
      const providers = (raw.providers as Record<string, unknown>) ?? {};
      const entries = Object.entries(providers).filter(([id]) => id !== "future");
      for (const [id, config] of entries) {
        if (isOverrideOnly(config)) continue;
        const c = config as Record<string, unknown>;
        const models = Array.isArray(c.models) ? c.models : [];
        const hasKey = keyedProviders.includes(id) ? " [key]" : "";
        lines.push(`  ${id}: ${models.length} model(s)${hasKey}`);
      }
    }
  } catch {
    // ignore
  }

  try {
    const client = new RunClient(GRPC_ADDR);
    const state = await client.getState();
    if (state.model) lines.push(`  Current model: ${state.model}`);
    if (state.contextWindow) lines.push(`  Context window: ${state.contextWindow.toLocaleString()}`);
  } catch {
    // agent not running
  }

  lines.unshift(`${keyCount} provider(s) with API key`);

  return { name: "Providers & models", status: "ok", lines };
}

// ── 6. Sessions ────────────────────────────────────────────────────────────

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
    if (sessions.length > 0) {
      lines.push(`${sessions.length} session(s)`);
      const recent = sessions
        .sort(
          (a, b) =>
            new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
        )
        .slice(0, 5);
      lines.push("Recent:");
      for (const s of recent) {
        const label = s.name || s.id.slice(0, 8);
        const model = s.model ? `  [${s.model}]` : "";
        lines.push(`  ${label}${model}`);
      }
    }
  } catch {
    // agent not running
  }

  return { name: "Sessions", status: "ok", lines };
}

// ── 7. Skills ──────────────────────────────────────────────────────────────

async function checkSkills(): Promise<CheckResult> {
  const lines: string[] = [];

  const installed = await getInstalledSkillIds();
  if (installed.size > 0) {
    lines.push(`${SKILLS_DIR}: ${[...installed].join(", ")}`);
  } else {
    lines.push("No skills installed.");
    const marker = fs.existsSync(SKILLS_DIR) ? "" : ` ${C.dim}(directory not found)${C.reset}`;
    lines.push(`  ${SKILLS_DIR}${marker}`);
  }

  // Check installed skills for updates against platform catalog
  if (installed.size > 0) {
    try {
      const platformUrl = await getPlatformUrl();
      const allSkills = await fetchSkills(platformUrl);
      const catalog = new Map(allSkills.map(s => [s.id, s]));
      let stale = 0;

      for (const id of installed) {
        const skill = catalog.get(id);
        if (!skill?.latest_version) continue;
        const localVer = await readSkillMdVersion(path.join(SKILLS_DIR, id, "SKILL.md"));
        if (localVer && localVer !== skill.latest_version) {
          lines.push(
            `  ${id}: ${localVer} ${C.dim}→${C.reset} ${skill.latest_version}`,
          );
          stale++;
        }
      }
      if (stale > 0) lines.push(`  ${stale} have updates — run ${C.bold}future skills update${C.reset}`);
    } catch {
      // offline or not logged in
    }
  }

  return {
    name: "Skills",
    status: installed.size > 0 ? "ok" : "warn",
    lines,
  };
}

// ── Output ─────────────────────────────────────────────────────────────────

function printResults(results: CheckResult[]): void {
  for (const r of results) {
    console.log(`${icon(r.status)} ${colorName(r.status, r.name)}`);
    for (const line of r.lines) {
      console.log(`      ${line}`);
    }
    console.log();
  }
}
