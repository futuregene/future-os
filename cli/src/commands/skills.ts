import { cp, mkdir, readdir, rm, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

// ── Paths ────────────────────────────────────────────────────────────────────

const HERMES_SKILLS = join(homedir(), ".hermes", "skills");

/** Resolve future-os skills/ directory relative to this CLI's source. */
function futureSkillsDir(): string {
  const currentFile = fileURLToPath(import.meta.url);
  // currentFile is .../future-os/cli/dist/commands/skills.js
  // Go up: commands → dist → cli → future-os → skills
  const cliRoot = resolve(dirname(currentFile), "..", "..", "..");
  return resolve(cliRoot, "skills");
}

// ── Types ────────────────────────────────────────────────────────────────────

export type SkillsCommand = "list" | "install" | "update" | "uninstall";

// ── Entry ────────────────────────────────────────────────────────────────────

export async function skills(command: SkillsCommand, args: string[]): Promise<void> {
  const name = args[0];

  if (command === "list") {
    await listSkills();
    return;
  }

  if (!name) {
    console.error(`Usage: future skills ${command} <skill-name>`);
    process.exitCode = 1;
    return;
  }

  if (command === "install") {
    await installSkill(name);
    return;
  }

  if (command === "update") {
    await updateSkill(name);
    return;
  }

  if (command === "uninstall") {
    await uninstallSkill(name);
    return;
  }
}

export function isSkillsCommand(command: string): command is SkillsCommand {
  return command === "list" || command === "install" || command === "update" || command === "uninstall";
}

// ── Implementation ───────────────────────────────────────────────────────────

async function listSkills(): Promise<void> {
  const src = futureSkillsDir();
  let entries: string[];

  try {
    entries = await readdir(src);
  } catch {
    console.error(`Skills directory not found: ${src}`);
    process.exitCode = 1;
    return;
  }

  const skillNames: string[] = [];
  for (const entry of entries) {
    const skillDir = join(src, entry);
    try {
      const info = await stat(skillDir);
      if (!info.isDirectory()) continue;
      await stat(join(skillDir, "SKILL.md"));
      skillNames.push(entry);
    } catch {
      // Ignore non-skill files and incomplete directories.
    }
  }

  if (skillNames.length === 0) {
    console.log("No skills available.");
    return;
  }

  const installed = new Set<string>();
  try {
    const hers = await readdir(HERMES_SKILLS);
    for (const d of hers) installed.add(d);
  } catch {
    // Hermes skills dir doesn't exist yet — none installed
  }

  for (const name of skillNames.sort()) {
    const marker = installed.has(name) ? "[installed]" : "";
    console.log(`  ${name.padEnd(30)} ${marker}`);
  }
  console.log(`\n${skillNames.length} skills available. Use "future skills install <name>" to install.`);
}

async function installSkill(name: string): Promise<void> {
  const src = join(futureSkillsDir(), name);
  const dest = join(HERMES_SKILLS, name);

  try {
    await stat(src);
  } catch {
    console.error(`Skill "${name}" not found in future-os/skills/.`);
    console.error(`Run "future skills list" to see available skills.`);
    process.exitCode = 1;
    return;
  }

  try {
    await stat(dest);
    console.log(`Skill "${name}" is already installed. Use "future skills update ${name}" to refresh.`);
    return;
  } catch {
    // Not installed — proceed
  }

  await mkdir(dest, { recursive: true });
  await cp(src, dest, { recursive: true });
  console.log(`Installed skill "${name}" → ${dest}`);
}

async function updateSkill(name: string): Promise<void> {
  const src = join(futureSkillsDir(), name);
  const dest = join(HERMES_SKILLS, name);

  try {
    await stat(src);
  } catch {
    console.error(`Skill "${name}" not found in future-os/skills/.`);
    process.exitCode = 1;
    return;
  }

  try {
    await stat(dest);
  } catch {
    console.log(`Skill "${name}" is not installed. Use "future skills install ${name}" first.`);
    return;
  }

  await rm(dest, { recursive: true, force: true });
  await mkdir(dest, { recursive: true });
  await cp(src, dest, { recursive: true });
  console.log(`Updated skill "${name}".`);
}

async function uninstallSkill(name: string): Promise<void> {
  const dest = join(HERMES_SKILLS, name);

  try {
    await stat(dest);
  } catch {
    console.log(`Skill "${name}" is not installed.`);
    return;
  }

  await rm(dest, { recursive: true, force: true });
  console.log(`Uninstalled skill "${name}".`);
}
