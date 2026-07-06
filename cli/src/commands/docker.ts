import { login, loadAuthFile, getFutureAuthEntry } from "./auth.js";
import { fetchSkills, getInstalledSkillIds, installBuiltinSkills as installBuiltin } from "./skills.js";
import { getPlatformUrl } from "../utils/platform.js";

// ── Types ────────────────────────────────────────────────────────────────────

interface CheckResult {
  name: string;
  status: "ok" | "issue";
  details: string[];
}

// ── Entry ────────────────────────────────────────────────────────────────────

export async function docker(fix: boolean): Promise<void> {
  console.log("🔍 Future Docker — checking environment...\n");

  // Check 1: Login status
  const loginResult = await checkLogin();

  // Check 2: Builtin skills
  const skillsResult = await checkBuiltinSkills();

  const results = [loginResult, skillsResult];
  printResults(results);

  const issues = results.filter(r => r.status === "issue");
  if (issues.length === 0) {
    console.log("✅ Everything looks good!");
    return;
  }

  if (!fix) {
    console.log(`\n${issues.length} issue(s) found. Run \`future doctor --fix\` to repair.`);
    return;
  }

  // --fix mode
  console.log("🔧 Fixing issues...\n");

  let loginOk = true;
  for (const result of results) {
    if (result.status === "ok") continue;
    if (result.name === "Login status") {
      loginOk = await tryFix("Starting login flow...", fixLogin);
    } else if (result.name.startsWith("Builtin skills")) {
      if (!loginOk) {
        console.log("  • Skipping skills install — login is required first.\n");
      } else {
        await tryFix("Installing builtin skills...", fixSkills);
      }
    }
  }

  console.log("\n✅ Done!");
}

// ── Checks ───────────────────────────────────────────────────────────────────

async function checkLogin(): Promise<CheckResult> {
  try {
    const authFile = await loadAuthFile();
    const auth = getFutureAuthEntry(authFile);
    if (auth?.key) {
      const platformUrl = auth.base_url
        ? auth.base_url.replace(/\/api\/?$/, "")
        : await getPlatformUrl();
      return {
        name: "Login status",
        status: "ok",
        details: [`Logged in to ${platformUrl}`],
      };
    }
    return {
      name: "Login status",
      status: "issue",
      details: ["Not logged in. No API key found."],
    };
  } catch {
    return {
      name: "Login status",
      status: "issue",
      details: ["Not logged in. Auth file not found."],
    };
  }
}

async function checkBuiltinSkills(): Promise<CheckResult> {
  let platformUrl: string;
  try {
    platformUrl = await getPlatformUrl();
  } catch {
    return {
      name: "Builtin skills",
      status: "issue",
      details: ["Cannot determine platform URL (not logged in?)."],
    };
  }

  let builtinSkills;
  try {
    builtinSkills = await fetchSkills(platformUrl, "builtin");
  } catch (err) {
    return {
      name: "Builtin skills",
      status: "issue",
      details: [`Failed to fetch builtin skills: ${err instanceof Error ? err.message : String(err)}`],
    };
  }

  if (builtinSkills.length === 0) {
    return {
      name: "Builtin skills",
      status: "ok",
      details: ["No builtin skills available."],
    };
  }

  const installed = await getInstalledSkillIds("app");
  const details: string[] = [];
  let hasIssues = false;

  for (const skill of builtinSkills) {
    const ver = skill.latest_version ? `v${skill.latest_version}` : "—";
    if (installed.has(skill.id)) {
      details.push(`${skill.id.padEnd(24)} ${ver} ✓`);
    } else {
      details.push(`${skill.id.padEnd(24)} not installed ✗`);
      hasIssues = true;
    }
  }

  return {
    name: `Builtin skills (${builtinSkills.length} available)`,
    status: hasIssues ? "issue" : "ok",
    details,
  };
}

// ── Fixers ───────────────────────────────────────────────────────────────────

async function tryFix(label: string, fn: () => Promise<void>): Promise<boolean> {
  console.log(`  • ${label}`);
  try {
    await fn();
    console.log();
    return true;
  } catch (err) {
    console.log(`     ✗ Failed: ${err instanceof Error ? err.message : String(err)}\n`);
    return false;
  }
}

async function fixLogin(): Promise<void> {
  await login();
}

async function fixSkills(): Promise<void> {
  await installBuiltin("app");
}

// ── Output ───────────────────────────────────────────────────────────────────

function printResults(results: CheckResult[]): void {
  let i = 1;
  for (const r of results) {
    const prefix = r.status === "ok" ? "✓" : "✗";
    console.log(`  ${i}. ${r.name}`);
    for (const d of r.details) {
      console.log(`     ${prefix} ${d}`);
    }
    console.log();
    i++;
  }
}
