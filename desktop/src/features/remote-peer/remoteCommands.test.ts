// @vitest-environment jsdom
import type { RemoteSkillChoice } from "./remoteCommands";
import { describe, expect, it } from "vitest";
import {
  buildRemoteSlashMenu,
  insertRemoteSkill,
  remoteSlashItems,
  slashQuery,
  withoutSlashWord,
} from "./remoteCommands";

/**
 * The slash word a draft is typing.
 *
 * Only a slash that begins a word counts: a path or a fraction in the middle of a
 * sentence must not open a menu over the message being written.
 */
describe("slashQuery", () => {
  it("reads the word after a slash that starts one", () => {
    expect(slashQuery("/co")).toBe("co");
    expect(slashQuery("hello /co")).toBe("co");
    expect(slashQuery("/")).toBe("");
    expect(slashQuery("hello /")).toBe("");
  });

  it("ignores a slash inside a word", () => {
    expect(slashQuery("analysis/effect-size.R")).toBeNull();
    expect(slashQuery("see docs/guide and")).toBeNull();
    expect(slashQuery("")).toBeNull();
    expect(slashQuery("no slash here")).toBeNull();
  });
});

/** The draft without the word the menu was opened by. */
describe("withoutSlashWord", () => {
  it("drops the slash word and the space that preceded it", () => {
    expect(withoutSlashWord("/co")).toBe("");
    expect(withoutSlashWord("hello /co")).toBe("hello");
    expect(withoutSlashWord("hello /")).toBe("hello");
  });

  it("leaves a slash that is part of the text alone", () => {
    expect(withoutSlashWord("analysis/effect-size.R")).toBe("analysis/effect-size.R");
  });
});

/**
 * Choosing a skill leaves its own name in the draft.
 *
 * The name is the host's invocation name — the only thing `/name` means to it —
 * and the trailing space is what lets the message carry on after it: the phone
 * inserts the same separator, because `/future-web帮我看一下` names a skill that
 * does not exist.
 */
describe("insertRemoteSkill", () => {
  it("replaces the typed word with `/<name>` and a separator", () => {
    expect(insertRemoteSkill("/rev", "future-peer-review")).toBe("/future-peer-review ");
    expect(insertRemoteSkill("帮我评审一下 /rev", "future-peer-review")).toBe("帮我评审一下 /future-peer-review ");
    // A bare slash is the same case: the menu opens on `/` alone.
    expect(insertRemoteSkill("/", "future-web")).toBe("/future-web ");
  });

  it("leaves a slash that is part of the text alone", () => {
    expect(insertRemoteSkill("analysis/effect-size.R", "future-web")).toBe("analysis/effect-size.R");
  });
});

describe("buildRemoteSlashMenu", () => {
  const commands = [
    { description: "", id: "compact", label: "Compact context", run: () => {} },
  ];
  const skills: RemoteSkillChoice[] = [
    { name: "future-peer-review", description: "Review manuscripts", descriptionZh: "评审稿件与基金申请书" },
    { name: "future-web", description: "Search the public web", descriptionZh: "搜索公开网页" },
  ];

  it("matches the id or the label, by substring", () => {
    expect(buildRemoteSlashMenu("co", commands, []).commands).toHaveLength(1);
    expect(buildRemoteSlashMenu("context", commands, []).commands).toHaveLength(1);
    // An empty query is "everything": the menu opens on `/` alone.
    expect(buildRemoteSlashMenu("", commands, skills).commands).toHaveLength(1);
    expect(buildRemoteSlashMenu("zzz", commands, []).commands).toHaveLength(0);
  });

  /**
   * A skill matches the name it is invoked by *or* the description in either
   * language: the row shows `future-peer-review` with a Chinese description, and
   * someone searching for「评审」has no other way to reach it.
   */
  it("matches a skill by its name or either description", () => {
    expect(buildRemoteSlashMenu("peer", commands, skills).skills.map(skill => skill.name))
      .toEqual(["future-peer-review"]);
    expect(buildRemoteSlashMenu("manuscript", commands, skills).skills.map(skill => skill.name))
      .toEqual(["future-peer-review"]);
    expect(buildRemoteSlashMenu("评审", commands, skills).skills.map(skill => skill.name))
      .toEqual(["future-peer-review"]);
    expect(buildRemoteSlashMenu("", commands, skills).skills).toHaveLength(2);
  });
});

/**
 * One flat order, shared by the menu and the keyboard.
 *
 * The commands lead and the skills follow, which is the order the menu draws and
 * the order the arrows walk; computing it twice is how the highlight ends up on a
 * row other than the one the reader sees.
 */
describe("remoteSlashItems", () => {
  it("is the commands and then the skills, in that order", () => {
    const commands = [{ description: "", id: "compact", label: "Compact context", run: () => {} }];
    const skills: RemoteSkillChoice[] = [{ name: "future-web", description: "", descriptionZh: null }];

    expect(remoteSlashItems({ commands, skills })).toEqual([
      { kind: "command", command: commands[0] },
      { kind: "skill", skill: skills[0] },
    ]);
  });
});
