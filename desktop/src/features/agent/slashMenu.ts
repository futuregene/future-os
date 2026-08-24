import type { ContextToolOption, SkillMentionOption } from "./MentionEditor";

export interface SlashMenuGroups {
  contextTools: ContextToolOption[];
  skills: SkillMentionOption[];
}

export function buildSlashMenuGroups(
  query: string,
  contextTools: ContextToolOption[],
  skills: SkillMentionOption[],
): SlashMenuGroups {
  const needle = query.toLowerCase();
  const matches = (values: Array<string | null | undefined>) =>
    values.some(value => (value ?? "").toLowerCase().includes(needle));
  return {
    contextTools: contextTools.filter(tool => matches([tool.name, tool.description, tool.searchText])),
    skills: skills
      .filter(skill => matches([skill.name, skill.description, skill.nameZh, skill.descriptionZh]))
      .slice(0, 20),
  };
}

export function hasMixedSlashResults(groups: SlashMenuGroups): boolean {
  return groups.contextTools.length > 0 && groups.skills.length > 0;
}
