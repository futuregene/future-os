import type { RemoteSkill } from "../../remote/types";

export interface TextSelection { start: number; end: number }
export interface SkillQuery { start: number; end: number; query: string }

/** Only standalone slash tokens; URLs, paths and non-collapsed selections are text. */
export function skillQuery(text: string, selection: TextSelection): SkillQuery | null {
  const { start: cursor, end } = selection;
  if (cursor !== end || cursor < 0 || cursor > text.length) return null;
  const before = text.slice(0, cursor);
  const match = /(?:^|\s)(\/[\p{L}\p{N}_.-]*)$/u.exec(before);
  if (!match) return null;
  const start = cursor - match[1]!.length;
  const tail = /^\S*/u.exec(text.slice(cursor))![0];
  const token = text.slice(start, cursor) + tail;
  if (!/^\/[\p{L}\p{N}_.-]*$/u.test(token)) return null;
  return { start, end: cursor + tail.length, query: text.slice(start + 1, cursor) };
}

/** Insert without destroying a selection or the surrounding draft. */
export function insertSkillSlash(text: string, selection: TextSelection) {
  const existing = skillQuery(text, selection);
  if (existing) return { text, selection };
  const start = Math.max(0, Math.min(selection.start, text.length));
  const prefix = start > 0 && !/\s/u.test(text[start - 1]!) ? " " : "";
  const suffix = start < text.length && !/\s/u.test(text[start]!) ? " " : "";
  const cursor = start + prefix.length + 1;
  return {
    text: `${text.slice(0, start)}${prefix}/${suffix}${text.slice(start)}`,
    selection: { start: cursor, end: cursor },
  };
}

export function completeSkill(text: string, query: SkillQuery, name: string) {
  const replacement = `/${name}`;
  const suffix = text.slice(query.end);
  const separator = suffix.startsWith(" ") ? "" : " ";
  const cursor = query.start + replacement.length + 1;
  return {
    text: text.slice(0, query.start) + replacement + separator + suffix,
    selection: { start: cursor, end: cursor },
  };
}

export function filterSkills(skills: RemoteSkill[], query: string) {
  const search = query.toLocaleLowerCase();
  return skills.filter(skill =>
    // Never insert malformed commands, even if an older/custom host sends one.
    typeof skill.name === "string" && /^[\p{L}\p{N}_.-]+$/u.test(skill.name)
    && [skill.name, skill.description, skill.nameZh, skill.descriptionZh]
      .some(value => value?.toLocaleLowerCase().includes(search)),
  );
}
