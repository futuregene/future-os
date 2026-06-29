import { createWriteStream } from "node:fs";
import { cp, mkdir, readdir, readFile, rename, rm, stat } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { execFile } from "node:child_process";

import { AUTH_FILE, DEFAULT_PLATFORM_URL } from "../constants.js";
import { isRecord } from "../utils/object.js";

// ── Paths ────────────────────────────────────────────────────────────────────

const APP_SKILLS = join(homedir(), ".future", "agent", "skills");
const GLOBAL_SKILLS = join(homedir(), ".agent", "skills");

function projectSkillsDir(): string {
  return join(process.cwd(), ".future", "agent", "skills");
}

// ── Types ────────────────────────────────────────────────────────────────────

export type SkillsCommand = "list" | "install" | "uninstall";
type Scope = "app" | "project" | "global";

interface SkillInfo {
  id: string;
  name: string;
  description: string;
  category: string;
  price: string;
  formats: string;
  limit: string;
  latest_version: string | null;
}

// ── Entry ────────────────────────────────────────────────────────────────────

export async function skills(command: SkillsCommand, args: string[]): Promise<void> {
  if (command === "list") {
    await listSkills();
    return;
  }

  const name = args[0];
  if (!name) {
    console.error(`Usage: future skills ${command} <skill-name> [--version <ver>] [--scope <app|project|global>]`);
    process.exitCode = 1;
    return;
  }

  const scope = parseScope(args);

  if (command === "install") {
    const versionIdx = args.indexOf("--version");
    const version = versionIdx !== -1 && versionIdx + 1 < args.length ? args[versionIdx + 1] : undefined;
    await installSkill(name, version, scope);
    return;
  }

  if (command === "uninstall") {
    await uninstallSkill(name, scope);
    return;
  }
}

export function isSkillsCommand(command: string): command is SkillsCommand {
  return command === "list" || command === "install" || command === "uninstall";
}

// ── Scope ────────────────────────────────────────────────────────────────────

function parseScope(args: string[]): Scope {
  const idx = args.indexOf("--scope");
  if (idx === -1) return "app";
  const val = args[idx + 1];
  if (val === "app" || val === "project" || val === "global") return val;
  console.error(`Invalid scope "${val}". Valid: app, project, global.`);
  process.exitCode = 1;
  return "app";
}

function skillsDirFor(scope: Scope): string {
  switch (scope) {
    case "app":     return APP_SKILLS;
    case "project": return projectSkillsDir();
    case "global":  return GLOBAL_SKILLS;
  }
}

function scopeLabel(scope: Scope, dir: string): string {
  if (scope === "project") return `${dir} (project)`;
  return dir;
}

// ── Platform URL ─────────────────────────────────────────────────────────────

async function getPlatformUrl(): Promise<string> {
  try {
    const raw = await readFile(AUTH_FILE, "utf8");
    const auth = JSON.parse(raw) as unknown;
    if (!isRecord(auth)) return DEFAULT_PLATFORM_URL;
    const future = auth["future"];
    if (!isRecord(future)) return DEFAULT_PLATFORM_URL;
    const url = typeof (future as Record<string, unknown>).platform_base_url === "string"
      ? (future as Record<string, unknown>).platform_base_url as string
      : DEFAULT_PLATFORM_URL;
    return url.replace(/\/$/, "");
  } catch {
    return DEFAULT_PLATFORM_URL;
  }
}

// ── Remote API ───────────────────────────────────────────────────────────────

async function fetchSkills(platformUrl: string): Promise<SkillInfo[]> {
  const resp = await fetch(`${platformUrl}/client/v1/skills`);
  if (!resp.ok) {
    throw new Error(`Failed to fetch skills: ${resp.status} ${resp.statusText}`);
  }
  const body = await resp.json() as { skills: SkillInfo[] };
  return body.skills ?? [];
}

async function downloadSkillZip(platformUrl: string, skillId: string, version: string): Promise<Readable> {
  const url = `${platformUrl}/client/v1/skills/${encodeURIComponent(skillId)}/versions/${encodeURIComponent(version)}/download`;
  const resp = await fetch(url);
  if (!resp.ok) {
    if (resp.status === 404) {
      throw new Error(`Skill version "${skillId}@${version}" not found.`);
    }
    throw new Error(`Failed to download skill: ${resp.status} ${resp.statusText}`);
  }
  if (!resp.body) {
    throw new Error("Empty response body");
  }
  return Readable.fromWeb(resp.body as any);
}

// ── Implementation ───────────────────────────────────────────────────────────

async function listSkills(): Promise<void> {
  const platformUrl = await getPlatformUrl();

  let skills: SkillInfo[];
  try {
    skills = await fetchSkills(platformUrl);
  } catch (err) {
    console.error(`Failed to fetch skills from ${platformUrl}/client/v1/skills`);
    console.error(err instanceof Error ? err.message : String(err));
    process.exitCode = 1;
    return;
  }

  if (skills.length === 0) {
    console.log("No skills available.");
    return;
  }

  // Check which skills are installed across all scopes
  const installed: Record<string, string[]> = {}; // skillId -> scopes
  for (const [scope, dir] of [["app", APP_SKILLS] as const, ["project", projectSkillsDir()] as const, ["global", GLOBAL_SKILLS] as const]) {
    try {
      const entries = await readdir(dir);
      for (const entry of entries) {
        (installed[entry] ??= []).push(scope);
      }
    } catch {
      // Skip nonexistent dirs
    }
  }

  for (const s of skills) {
    const scopes = installed[s.id];
    const marker = scopes && scopes.length > 0 ? `[installed: ${scopes.join(", ")}]` : "";
    const ver = s.latest_version ? `v${s.latest_version}` : "(no version)";
    console.log(`  ${s.id.padEnd(30)} ${ver.padEnd(12)} ${marker}`);
    console.log(`    ${s.name}  |  ${s.price}  |  ${s.formats}`);
  }
  console.log(`\n${skills.length} skills available. Use "future skills install <name>" to install.`);
}

async function installSkill(skillId: string, version?: string, scope: Scope = "app"): Promise<void> {
  const platformUrl = await getPlatformUrl();

  // Fetch skill metadata to get latest_version if version not specified
  if (!version) {
    let skills: SkillInfo[];
    try {
      skills = await fetchSkills(platformUrl);
    } catch (err) {
      console.error("Failed to fetch skill metadata.");
      console.error(err instanceof Error ? err.message : String(err));
      process.exitCode = 1;
      return;
    }
    const skillMeta = skills.find(s => s.id === skillId);
    if (!skillMeta) {
      console.error(`Skill "${skillId}" not found in catalog.`);
      console.error(`Run "future skills list" to see available skills.`);
      process.exitCode = 1;
      return;
    }
    if (!skillMeta.latest_version) {
      console.error(`Skill "${skillId}" has no versions available.`);
      process.exitCode = 1;
      return;
    }
    version = skillMeta.latest_version;
  }

  const skillsDir = skillsDirFor(scope);
  const dest = join(skillsDir, skillId);
  let isUpdate = false;
  try {
    await stat(dest);
    isUpdate = true;
  } catch {
    // Not installed — fresh install
  }

  // Download the zip
  console.log(`Downloading ${skillId} v${version}...`);
  const tmpZip = join(tmpdir(), `future-skill-${skillId}-${version}.zip`);
  let zipStream: Readable;
  try {
    zipStream = await downloadSkillZip(platformUrl, skillId, version!);
  } catch (err) {
    console.error(err instanceof Error ? err.message : String(err));
    process.exitCode = 1;
    return;
  }

  try {
    // Write zip to temp file
    const fileStream = createWriteStream(tmpZip);
    await pipeline(zipStream, fileStream);

    // Prepare destination
    if (isUpdate) {
      await rm(dest, { recursive: true, force: true });
    }
    await mkdir(dest, { recursive: true });

    // Extract zip
    await unzip(tmpZip, dest);

    // If zip contents are wrapped in a single subdirectory, flatten it
    await flattenSingleSubdir(dest);

    console.log(`${isUpdate ? "Updated" : "Installed"} skill "${skillId}" v${version} → ${scopeLabel(scope, dest)}`);
  } finally {
    // Clean up temp file
    try { await rm(tmpZip, { force: true }); } catch { /* ignore */ }
  }
}

async function uninstallSkill(skillId: string, scope: Scope = "app"): Promise<void> {
  const skillsDir = skillsDirFor(scope);
  const dest = join(skillsDir, skillId);

  try {
    await stat(dest);
  } catch {
    console.log(`Skill "${skillId}" is not installed${scope !== "app" ? ` (${scope})` : ""}.`);
    return;
  }

  await rm(dest, { recursive: true, force: true });
  console.log(`Uninstalled skill "${skillId}" from ${scopeLabel(scope, skillsDir)}.`);
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function unzip(zipPath: string, destDir: string): Promise<void> {
  return new Promise((resolve, reject) => {
    execFile("unzip", ["-o", zipPath, "-d", destDir], (err, _stdout, stderr) => {
      if (err) {
        reject(new Error(`unzip failed: ${String(stderr || err.message)}`));
      } else {
        resolve();
      }
    });
  });
}

/**
 * If the extracted directory contains exactly one subdirectory and nothing else,
 * move its contents up one level (flatten). This handles zips that wrap
 * their contents in a top-level directory (e.g. paper-summary/SKILL.md → SKILL.md).
 */
async function flattenSingleSubdir(dir: string): Promise<void> {
  let entries: string[];
  try {
    entries = await readdir(dir);
  } catch {
    return;
  }
  if (entries.length !== 1) return;

  const single = join(dir, entries[0]);
  let info;
  try {
    info = await stat(single);
  } catch {
    return;
  }
  if (!info.isDirectory()) return;

  // Move contents of single subdir up to dir
  const children = await readdir(single);
  for (const child of children) {
    // Remove any existing item at destination with same name
    await rm(join(dir, child), { recursive: true, force: true }).catch(() => {});
    await renameAcrossDevice(join(single, child), join(dir, child));
  }
  await rm(single, { recursive: true, force: true });
}

/** Cross-device rename using copy + delete fallback. */
async function renameAcrossDevice(src: string, dest: string): Promise<void> {
  try {
    await rename(src, dest);
  } catch {
    await cp(src, dest, { recursive: true });
    await rm(src, { recursive: true, force: true });
  }
}
