import type { ContextToolOption, SkillMentionOption } from "./MentionEditor";
import { describe, expect, it } from "vitest";
import { buildSlashMenuGroups, hasMixedSlashResults } from "./slashMenu";

const compact: ContextToolOption = {
  id: "compact",
  name: "压缩",
  description: "压缩此对话的上下文",
  searchText: "compact compaction context 压缩 上下文",
};
const skill: SkillMentionOption = {
  name: "review-agent",
  description: "Review code changes",
  nameZh: "代码审查",
  descriptionZh: "检查代码变更",
};

describe("slash menu grouping", () => {
  it("matches the compact context tool in Chinese and English", () => {
    expect(buildSlashMenuGroups("压缩", [compact], [skill]).contextTools).toEqual([compact]);
    expect(buildSlashMenuGroups("compact", [compact], [skill]).contextTools).toEqual([compact]);
  });

  it("keeps context tools above skills and marks only mixed results for a separator", () => {
    const mixed = buildSlashMenuGroups("", [compact], [skill]);
    expect(mixed).toEqual({ contextTools: [compact], skills: [skill] });
    expect(hasMixedSlashResults(mixed)).toBe(true);

    const toolOnly = buildSlashMenuGroups("compact", [compact], [skill]);
    expect(toolOnly.skills).toEqual([]);
    expect(hasMixedSlashResults(toolOnly)).toBe(false);

    const skillOnly = buildSlashMenuGroups("review", [compact], [skill]);
    expect(skillOnly.contextTools).toEqual([]);
    expect(hasMixedSlashResults(skillOnly)).toBe(false);
  });

  it("matches skills by localized metadata without changing their slash name", () => {
    const groups = buildSlashMenuGroups("代码", [compact], [skill]);
    expect(groups.skills).toEqual([skill]);
    expect(groups.skills[0]?.name).toBe("review-agent");
  });
});
