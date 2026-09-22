// Builds the skill roster the recommender ranks against.
//
// Source of truth is the `skills` submodule: skills/builtin/*/SKILL.md (15) and
// skills/third-party/*/SKILL.md (126). skills.json only enriches the entries with the
// Chinese name/description the UI shows.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";

const FRONTMATTER_RE = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/;

/** Minimal YAML frontmatter reader: `key: value` and `key: >` / `key: |` block scalars. */
export function parseFrontmatter(md) {
  const match = FRONTMATTER_RE.exec(md);
  if (!match) return { fields: {}, body: md };
  const lines = match[1].split(/\r?\n/);
  const fields = {};
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const kv = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(line);
    if (!kv) continue;
    const key = kv[1];
    const raw = kv[2].trim();
    if (raw === ">" || raw === "|" || raw === ">-" || raw === "|-") {
      const collected = [];
      while (i + 1 < lines.length && (lines[i + 1].trim() === "" || /^\s+/.test(lines[i + 1]))) {
        collected.push(lines[i + 1].trim());
        i += 1;
      }
      fields[key] = collected.join(" ").replace(/\s+/g, " ").trim();
    } else {
      fields[key] = raw.replace(/^["']|["']$/g, "");
    }
  }
  return { fields, body: md.slice(match[0].length) };
}

const oneLine = (text) => (text || "").replace(/\s+/g, " ").trim();

export function loadRoster(skillsRoot, { excerptChars = 900, descChars = 220 } = {}) {
  const catalogPath = path.join(skillsRoot, "skills.json");
  const catalog = existsSync(catalogPath)
    ? JSON.parse(readFileSync(catalogPath, "utf8"))
    : {};

  const skills = [];
  for (const [folder, builtin] of [
    ["builtin", true],
    ["third-party", false],
  ]) {
    const dir = path.join(skillsRoot, folder);
    if (!existsSync(dir)) continue;
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (!entry.isDirectory()) continue;
      const skillFile = path.join(dir, entry.name, "SKILL.md");
      if (!existsSync(skillFile)) continue;
      const { fields, body } = parseFrontmatter(readFileSync(skillFile, "utf8"));
      const meta = catalog[entry.name] || catalog[fields.name] || {};
      skills.push({
        name: fields.name || entry.name,
        folder: `${builtin ? "builtin" : "third-party"}/${entry.name}`,
        builtin,
        category: meta.category || fields.category || "other",
        nameZh: meta.name_zh || "",
        description: oneLine(fields.description),
        descriptionZh: oneLine(meta.description_zh),
        excerpt: oneLine(body).slice(0, excerptChars),
        // Same opening, but with the markdown intact: `excerpt` collapses every newline, so
        // headings, lists and code fences arrive as one paragraph. Kept side by side so the
        // two shapes can be measured against each other (bench/excerpt-shape.mjs).
        excerptRaw: body.trim().slice(0, excerptChars),
        // What stage 1 shows the model for this skill: one line, not truncated to the
        // 60 chars a harness index would use.
        indexLine: oneLine(fields.description).slice(0, descChars),
      });
    }
  }

  return {
    skills,
    byName: new Map(skills.map((skill) => [skill.name, skill])),
    builtinCount: skills.filter((s) => s.builtin).length,
    thirdPartyCount: skills.filter((s) => !s.builtin).length,
  };
}
