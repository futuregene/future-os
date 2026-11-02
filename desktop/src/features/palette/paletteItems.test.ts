import type { PaletteCommand, PaletteTarget } from "./paletteItems";
import { describe, expect, it, vi } from "vitest";
import { entryRank, flattenSections, matchRank, paletteSections } from "./paletteItems";

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

describe("entryRank", () => {
  it("finds a row by an alias when the label does not contain the word", () => {
    // "keybind" is nobody's label; it is how a reader asks for the page that
    // edits bindings.
    expect(entryRank("Keyboard shortcut settings", ["keybind", "hotkey"], "keybind")).toBeGreaterThanOrEqual(0);
    // A row with no such alias, and a row whose aliases are unrelated, are out.
    expect(entryRank("New conversation", [], "keybind")).toBe(-1);
    expect(entryRank("New conversation", ["hotkey"], "keybind")).toBe(-1);
  });

  it("ranks everything equally when nothing is typed", () => {
    // The palette's first screen is the list itself, in the caller's order; there is
    // no query to rank against, so every row ties.
    expect(entryRank("New conversation", [], "")).toBe(0);
    expect(entryRank("", ["anything"], "")).toBe(0);
  });

  it("takes the best of several aliases rather than the first that matches", () => {
    // `xxquill` only contains the word mid-word (rank 2); `quill` starts with it
    // (rank 0). The alias tier is a flat +3 on whichever hit is best, so the row still
    // sorts above a row that does not match at all — and below a label match.
    expect(entryRank("Other", ["xxquill", "quill"], "quill")).toBe(3);
    expect(entryRank("Other", ["quill", "xxquill"], "quill")).toBe(3);
  });

  it("ranks a label match above an alias match", () => {
    const byLabel = entryRank("Keyboard shortcut settings", [], "shortcut");
    const byAlias = entryRank("Open settings", ["shortcut settings"], "shortcut");
    expect(byLabel).toBeLessThan(byAlias);
  });
});

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

  it("finds a command through its aliases", () => {
    const settings: PaletteCommand = {
      aliases: ["keybind", "hotkey"],
      category: "settings",
      id: "shortcuts.settings",
      label: "Keyboard shortcut settings",
    };
    // Both halves of the requirement: the word in the label, and the word in the
    // reader's head ("keybind", "hotkey") that the label never uses.
    for (const query of ["shortcut", "keybind", "hotkey", "KEYBIND"])
      expect(query.length > 0 ? paletteSections({ commands: [settings], labels, query, targets: [] }) : []).toHaveLength(1);
    // An unrelated word still finds nothing.
    expect(paletteSections({ commands: [settings], labels, query: "zzz", targets: [] })).toEqual([]);
  });

  it("flattens to the reading order the arrow keys walk", () => {
    const sections = query("", [command("A"), command("B")], [target("C"), target("D", "workspace")]);
    expect(flattenSections(sections).map(entry => entry.label)).toEqual(["A", "B", "C", "D"]);
  });
});
