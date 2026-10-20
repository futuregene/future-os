import type { PaletteCommand, PaletteTarget } from "./paletteItems";
import { describe, expect, it, vi } from "vitest";
import { flattenSections, matchRank, paletteSections } from "./paletteItems";

const labels = {
  category: (category: string) => `cat:${category}`,
  commands: "Commands",
  conversations: "Conversations",
  workspaces: "Workspaces",
};

function command(label: string, id = label): PaletteCommand {
  return { category: "chat", id: id as PaletteCommand["id"], label };
}

function target(title: string, kind: PaletteTarget["kind"] = "conversation", subtitle?: string): PaletteTarget {
  return { key: `${kind}:${title}`, kind, run: vi.fn(), subtitle, title };
}

function query(text: string, commands: PaletteCommand[], targets: PaletteTarget[] = []) {
  return paletteSections({ commands, labels, query: text, targets });
}

describe("matchRank", () => {
  it("ranks a prefix above a word start above a mid-word hit", () => {
    expect(matchRank("Command palette", "com")).toBe(0);
    expect(matchRank("Open the command palette", "pal")).toBe(1);
    expect(matchRank("Open the command palette", "mm")).toBe(2);
    expect(matchRank("Open the command palette", "zz")).toBe(-1);
  });

  it("is case-insensitive and empty for an empty query", () => {
    expect(matchRank("Command", "COMMAND")).toBe(0);
    expect(matchRank("Command", "")).toBe(0);
  });
});

describe("paletteSections", () => {
  it("keeps the table's and the rail's order when nothing is typed", () => {
    const sections = query("", [command("Alpha"), command("Beta")], [target("Third"), target("Fourth")]);
    expect(sections.map(section => section.key)).toEqual(["commands", "conversations"]);
    expect(sections[0]!.entries.map(entry => entry.label)).toEqual(["Alpha", "Beta"]);
    expect(sections[1]!.entries.map(entry => entry.label)).toEqual(["Third", "Fourth"]);
  });

  it("separates commands from conversations instead of ranking them together", () => {
    // A chat called "terminal notes" must not outrank the command named exactly
    // "terminal": two kinds of answer, two sections.
    const sections = query("terminal", [command("Show the terminal")], [target("terminal notes")]);
    expect(sections.map(section => section.key)).toEqual(["commands", "conversations"]);
    expect(sections[0]!.entries[0]!.label).toBe("Show the terminal");
  });

  it("drops a section with no match rather than heading an empty one", () => {
    const sections = query("zzz", [command("Alpha")], [target("Beta")]);
    expect(sections).toEqual([]);
  });

  it("ranks a prefix hit above a mid-word one inside a section", () => {
    const sections = query("pal", [command("Open the command palette"), command("Palette settings")]);
    expect(sections[0]!.entries.map(entry => entry.label)).toEqual(["Palette settings", "Open the command palette"]);
  });

  it("gives a command its category and a target its location as the second line", () => {
    const sections = query("", [command("Alpha")], [target("Beta", "conversation", "Workspace A")]);
    expect(sections[0]!.entries[0]!.hint).toBe("cat:chat");
    expect(sections[1]!.entries[0]!.hint).toBe("Workspace A");
  });

  it("keeps a command's id on the row so the view can run it", () => {
    const sections = query("", [command("Alpha", "chat.new")]);
    expect(sections[0]!.entries[0]!.commandId).toBe("chat.new");
    expect(sections[0]!.entries[0]!.run).toBeUndefined();
  });

  it("keeps a target's action on the row instead", () => {
    const run = vi.fn();
    const sections = query("", [], [{ key: "k", kind: "workspace", run, title: "W" }]);
    expect(sections[0]!.entries[0]!.commandId).toBeUndefined();
    expect(sections[0]!.entries[0]!.run).toBe(run);
  });

  it("lists workspaces as their own section, after conversations", () => {
    const sections = query("", [], [target("A chat"), target("A workspace", "workspace", "/tmp/w")]);
    expect(sections.map(section => section.key)).toEqual(["conversations", "workspaces"]);
  });

  it("trims the query before matching", () => {
    expect(query("  alpha  ", [command("Alpha")])[0]!.entries).toHaveLength(1);
  });

  it("flattens to the reading order the arrow keys walk", () => {
    const sections = query("", [command("A"), command("B")], [target("C"), target("D", "workspace")]);
    expect(flattenSections(sections).map(entry => entry.label)).toEqual(["A", "B", "C", "D"]);
  });
});
