import { createWriteStream } from "node:fs";
import { cp, mkdir, readdir, readFile, rename, rm, stat } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { execFile } from "node:child_process";

import { getPlatformUrl } from "../utils/platform.js";

// ── Paths ──────────────────────────────────────────────────────────────────

export const SKILLS_DIR = join(homedir(), ".future", "agent", "skills");

// ── Types ──────────────────────────────────────────────────────────────────

export type SkillsCommand = "list" | "install" | "uninstall" | "install-builtin" | "update";

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

// ── Entry ──────────────────────────────────────────────────────────────────

export async function skills(command: SkillsCommand, args: string[]): Promise<void> {
  if (command === "list") {
    await listSkills();
    return;
  }

  if (command === "install-builtin") {
    await installBuiltinSkills();
    return;
  }

  if (command === "update") {
    await updateSkills();
    return;
  }

  if (command === "install") {
    const name = args[0];
    if (!name) {
      // No name given — install all builtin skills (same as install-builtin)
      await installBuiltinSkills();
      return;
    }
    const versionIdx = args.indexOf("--version");
    const version = versionIdx !== -1 && versionIdx + 1 < args.length ? args[versionIdx + 1] : undefined;
    await installSkill(name, version);
    return;
  }

  const name = args[0];
  if (!name) {
    console.error(`Usage: future skills ${command} <skill-name>`);
    process.exitCode = 1;
    return;
  }

  if (command === "uninstall") {
    await uninstallSkill(name);
    return;
  }
}

export function isSkillsCommand(command: string): command is SkillsCommand {
  return command === "list" || command === "install" || command === "uninstall" || command === "install-builtin" || command === "update";
}

// ── Remote API ─────────────────────────────────────────────────────────────

export async function fetchSkills(platformUrl: string): Promise<SkillInfo[]> {
  const url = `${platformUrl}/client/v1/skills`;
  const resp = await fetch(url);
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

// ── Implementation ─────────────────────────────────────────────────────────

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

  // Check which skills are installed
  const installed: Record<string, string> = {};
  try {
    const entries = await readdir(SKILLS_DIR);
    for (const entry of entries) {
      try {
        const ver = await readSkillMdVersion(join(SKILLS_DIR, entry, "SKILL.md"));
        if (ver) installed[entry] = ver;
      } catch {
        // skip
      }
    }
  } catch {
    // Dir doesn't exist
  }

  const idWidth = Math.min(36, Math.max(12, ...skills.map(s => s.id.length)));
  const verWidth = Math.max(10, ...skills.map(s => (s.latest_version ? `v${s.latest_version}` : "—").length));
  const instWidth = skills.reduce((max, s) => {
    const marker = installed[s.id] ? `v${installed[s.id]}` : "—";
    return Math.max(max, marker.length);
  }, 9);

  const DESC_MAX = 48;
  const descWidth = Math.min(DESC_MAX, Math.max(12, ...skills.map(s => s.description.length)));

  console.log(`  ${"NAME".padEnd(idWidth)} ${"LATEST".padEnd(verWidth)} ${"INSTALLED".padEnd(instWidth)} DESCRIPTION`);
  console.log(`  ${"—".repeat(idWidth)} ${"—".repeat(verWidth)} ${"—".repeat(instWidth)} ${"—".repeat(descWidth)}`);

  for (const s of skills) {
    const marker = installed[s.id] ? `v${installed[s.id]}` : "—";
    const ver = s.latest_version ? `v${s.latest_version}` : "—";
    const desc = s.description.length > DESC_MAX ? s.description.slice(0, DESC_MAX - 1) + "…" : s.description;
    console.log(`  ${s.id.padEnd(idWidth)} ${ver.padEnd(verWidth)} ${marker.padEnd(instWidth)} ${desc.padEnd(descWidth)}`);
  }
  console.log(`\n${skills.length} skills available. Use "future skills install <name>" to install.`);
}

async function installSkill(skillId: string, version?: string): Promise<void> {
  const platformUrl = await getPlatformUrl();

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

  const dest = join(SKILLS_DIR, skillId);
  let isUpdate = false;
  try {
    await stat(dest);
    isUpdate = true;
  } catch {
    // fresh install
  }

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
    const fileStream = createWriteStream(tmpZip);
    await pipeline(zipStream, fileStream);

    if (isUpdate) {
      await rm(dest, { recursive: true, force: true });
    }
    await mkdir(dest, { recursive: true });
    await unzip(tmpZip, dest);
    await flattenSingleSubdir(dest);

    console.log(`${isUpdate ? "Updated" : "Installed"} skill "${skillId}" v${version} → ${dest}`);
  } finally {
    try { await rm(tmpZip, { force: true }); } catch { /* ignore */ }
  }
}

export async function installBuiltinSkills(): Promise<void> {
  const platformUrl = await getPlatformUrl();

  let skills: SkillInfo[];
  try {
    skills = (await fetchSkills(platformUrl)).filter(s => s.id.startsWith("future-"));
  } catch (err) {
    console.error("Failed to fetch builtin skills.");
    console.error(err instanceof Error ? err.message : String(err));
    process.exitCode = 1;
    return;
  }

  if (skills.length === 0) {
    console.log("No builtin skills available.");
    return;
  }

  const installed = await getInstalledSkillIds();

  const toInstall = skills.filter(s => !installed.has(s.id));

  if (toInstall.length === 0) {
    console.log(`All ${skills.length} builtin skills are already installed.`);
    return;
  }

  const skipped = skills.length - toInstall.length;
  console.log(`Installing ${toInstall.length} builtin skills${skipped > 0 ? ` (${skipped} already installed)` : ""}...`);

  for (const skill of toInstall) {
    if (!skill.latest_version) {
      console.log(`  Skipping ${skill.id} — no version available.`);
      continue;
    }
    try {
      await installSkill(skill.id, skill.latest_version);
    } catch (err) {
      console.error(`  Failed to install ${skill.id}: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  console.log(`Done. ${toInstall.length} skills installed.`);
}

/** Update all installed skills to their latest versions. */
async function updateSkills(): Promise<void> {
  const platformUrl = await getPlatformUrl();
  console.log(`Fetching skill catalog from ${platformUrl}...`);
  const skills = await fetchSkills(platformUrl);
  if (skills.length === 0) {
    console.log("No skills available.");
    return;
  }

  const installed = await getInstalledSkillIds();
  if (installed.size === 0) {
    console.log("No skills installed.");
    return;
  }

  let updated = 0;
  let upToDate = 0;

  for (const skill of skills) {
    if (!installed.has(skill.id)) continue;
    if (!skill.latest_version) continue;

    const skillMdPath = join(SKILLS_DIR, skill.id, "SKILL.md");
    const localVer = await readSkillMdVersion(skillMdPath);
    if (!localVer || localVer === skill.latest_version) {
      upToDate++;
      continue;
    }

    console.log(`  ${skill.id}: ${localVer} → ${skill.latest_version}`);
    try {
      await installSkill(skill.id, skill.latest_version);
      updated++;
    } catch (err) {
      console.error(`  Failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  if (updated === 0) {
    console.log(`${upToDate} skill(s) already up to date.`);
  } else {
    console.log(`Updated ${updated} skill(s), ${upToDate} already up to date.`);
  }
}

export async function getInstalledSkillIds(): Promise<Set<string>> {
  const ids = new Set<string>();
  try {
    const entries = await readdir(SKILLS_DIR);
    for (const entry of entries) {
      try {
        await stat(join(SKILLS_DIR, entry, "SKILL.md"));
        ids.add(entry);
      } catch {
        // No SKILL.md — skip
      }
    }
  } catch {
    // Dir doesn't exist — nothing installed
  }
  return ids;
}

async function uninstallSkill(skillId: string): Promise<void> {
  const dest = join(SKILLS_DIR, skillId);

  try {
    await stat(dest);
  } catch {
    console.log(`Skill "${skillId}" is not installed.`);
    return;
  }

  await rm(dest, { recursive: true, force: true });
  console.log(`Uninstalled skill "${skillId}" from ${SKILLS_DIR}.`);
}

// ── Helpers ────────────────────────────────────────────────────────────────

/** Read YAML frontmatter from a SKILL.md and extract the version field. */
export async function readSkillMdVersion(skillMdPath: string): Promise<string | null> {
  let text: string;
  try {
    text = await readFile(skillMdPath, "utf8");
  } catch {
    return null;
  }

  const trimmed = text.trimStart();
  if (!trimmed.startsWith("---")) return null;

  const rest = trimmed.slice(3);
  const endIdx = Math.max(
    rest.indexOf("\n---"),
    rest.indexOf("---"),
  );
  if (endIdx === -1) return null;

  const frontmatter = rest.slice(0, endIdx);

  const lines = frontmatter.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trim();
    if (!t || t.startsWith("#")) continue;

    // Direct version field: version: 1.0.0
    const vm = t.match(/^version:\s*(.+)$/);
    if (vm) return unquote(vm[1].trim());

    // Metadata JSON (single line): metadata: {"version": "1.0", ...}
    // or YAML block: metadata:\n  version: "1.0"
    const mm = t.match(/^metadata:\s*(.*)$/);
    if (mm) {
      const rest = mm[1];
      if (rest) {
        // Try JSON first
        try {
          const meta = JSON.parse(rest);
          if (meta.version) return String(meta.version);
        } catch {
          // not JSON, maybe inline YAML like metadata: version: "1.0"
          const inline = rest.match(/version:\s*(.+)$/);
          if (inline) return unquote(inline[1].trim());
        }
      }
      // YAML block: scan indented lines for version:
      for (let j = i + 1; j < lines.length; j++) {
        const sub = lines[j];
        if (sub.trim().startsWith("#")) continue;
        if (!sub.startsWith(" ") && !sub.startsWith("\t")) break; // end of block
        const sv = sub.match(/version:\s*(.+)$/);
        if (sv) return unquote(sv[1].trim());
      }
    }
  }
  return null;
}

function unquote(val: string): string {
  if ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) {
    val = val.slice(1, -1);
  }
  return val || "";
}
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
 * move its contents up one level (flatten).
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

  const children = await readdir(single);
  for (const child of children) {
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
