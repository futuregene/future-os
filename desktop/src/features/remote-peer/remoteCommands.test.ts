// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { buildRemoteSlashMenu, slashQuery, withoutSlashWord } from "./remoteCommands";

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

describe("buildRemoteSlashMenu", () => {
  const commands = [
    { description: "", id: "compact", label: "Compact context", run: () => {} },
  ];

  it("matches the id or the label, by substring", () => {
    expect(buildRemoteSlashMenu("co", commands).commands).toHaveLength(1);
    expect(buildRemoteSlashMenu("context", commands).commands).toHaveLength(1);
    // An empty query is "everything": the menu opens on `/` alone.
    expect(buildRemoteSlashMenu("", commands).commands).toHaveLength(1);
    expect(buildRemoteSlashMenu("zzz", commands).commands).toHaveLength(0);
  });
});
