/**
 * What the command palette lists, in what order, for what the user typed.
 *
 * Pure, because this is the part worth pinning: which entries a query matches,
 * how ties are ordered, and that a command and a conversation are never confused
 * for one another. The dialog itself is layout and key handling.
 *
 * The palette is one entry that answers two different questions — *what can I
 * do* (a command) and *where can I go* (a conversation, a workspace). They are
 * kept as separate sections rather than interleaved: a reader who types
 * "terminal" wants the command, and a reader who types a conversation's title
 * wants the conversation; one ranking over both would let a chat called
 * "terminal notes" outrank the command of that name.
 *
 * `mod+shift+f` is deliberately not this: it filters the rail's list in place,
 * while this navigates. Two different questions, two different keys.
 */
import type { CommandCategory, CommandId } from "../keyboard/catalog";

/** A command the palette may run. */
export interface PaletteCommand {
  id: CommandId;
  /** Already translated. */
  label: string;
  category: CommandCategory;
  /**
   * Extra search terms that do not appear in the label — "keybind" for the
   * shortcut settings page, "hotkey", "keys".
   *
   * A palette is searched by the word in the user's head, and that word is often
   * not the word the label happens to use. Aliases only ever *find* a row; they
   * never rank it above a label match.
   */
  aliases?: readonly string[];
}

/** Somewhere the palette may go. */
export interface PaletteTarget {
  /** Unique within the palette — the row key the list is built from. */
  key: string;
  kind: "conversation" | "workspace";
  title: string;
  /** The machine or workspace it lives in, for the second line. */
  subtitle?: string;
  run: () => void;
}

/** One selectable row: either a command to run or a target to open. */
export interface PaletteEntry {
  key: string;
  label: string;
  /** Second line: the category for a command, the location for a target. */
  hint?: string;
  commandId?: CommandId;
  /** Extra search terms; see `PaletteCommand.aliases`. */
  aliases?: readonly string[];
  run?: () => void;
}

export interface PaletteSection {
  key: "commands" | "conversations" | "workspaces";
  title: string;
  entries: PaletteEntry[];
}

/** Everything a section's title needs, so this module stays free of i18n. */
export interface PaletteLabels {
  commands: string;
  conversations: string;
  workspaces: string;
  /** Resolves a command's category name for its second line. */
  category: (category: CommandCategory) => string;
}

/**
 * Rank one label against a query: `0` for a prefix, `1` for the start of a word
 * inside it, `2` for anywhere, `-1` for no match.
 *
 * Ranking by *where* the match is, rather than by how long the label is, is what
 * makes a prefix hit win over a mid-word one; inside one rank the caller's order
 * is kept, so the table's order and the rail's order survive.
 */
/**
 * The rank of one row: its label's, or one tier worse for an alias-only hit.
 *
 * The penalty is what keeps `shortcut` finding the command actually *named*
 * "Keyboard shortcut settings" before a command that merely lists the word as an
 * alias.
 */
export function entryRank(label: string, aliases: readonly string[], query: string): number {
  if (query.length === 0)
    return 0;
  const own = matchRank(label, query);
  if (own >= 0)
    return own;
  const alias = aliases
    .map(candidate => matchRank(candidate, query))
    .filter(rank => rank >= 0)
    .sort((left, right) => left - right)[0];
  return alias === undefined ? -1 : alias + 3;
}

export function matchRank(label: string, query: string): number {
  if (query.length === 0)
    return 0;
  const haystack = label.toLowerCase();
  const index = haystack.indexOf(query.toLowerCase());
  if (index === 0)
    return 0;
  if (index > 0)
    return /[\s\-/.:(]/.test(haystack[index - 1] ?? "") ? 1 : 2;
  return -1;
}

function rankBy(entries: PaletteEntry[], query: string): PaletteEntry[] {
  if (query.length === 0)
    return entries;
  return entries
    .map(entry => ({ entry, rank: entryRank(entry.label, aliasesOf(entry), query) }))
    .filter(scored => scored.rank >= 0)
    .sort((left, right) => left.rank - right.rank)
    .map(scored => scored.entry);
}

/**
 * The palette's contents for a query.
 *
 * Empty sections are dropped, so the dialog shows "no matches" only when every
 * section is empty — a query that matches three conversations and no command
 * must not print a "Commands" heading over nothing.
 */
export function paletteSections(
  { commands, labels, query, targets }: {
    commands: readonly PaletteCommand[];
    labels: PaletteLabels;
    query: string;
    targets: readonly PaletteTarget[];
  },
): PaletteSection[] {
  const trimmed = query.trim().toLowerCase();
  const toEntry = (target: PaletteTarget): PaletteEntry => ({
    hint: target.subtitle,
    key: target.key,
    label: target.title,
    run: target.run,
  });
  const sections: PaletteSection[] = [
    {
      entries: rankBy(commands.map(command => ({
        aliases: command.aliases,
        commandId: command.id,
        hint: labels.category(command.category),
        key: command.id,
        label: command.label,
      })), trimmed),
      key: "commands",
      title: labels.commands,
    },
    {
      entries: rankBy(targets.filter(target => target.kind === "conversation").map(toEntry), trimmed),
      key: "conversations",
      title: labels.conversations,
    },
    {
      entries: rankBy(targets.filter(target => target.kind === "workspace").map(toEntry), trimmed),
      key: "workspaces",
      title: labels.workspaces,
    },
  ];
  return sections.filter(section => section.entries.length > 0);
}

/** A row's aliases, or none — a target has no keyword list of its own. */
function aliasesOf(entry: PaletteEntry): readonly string[] {
  return entry.aliases ?? [];
}

/** Every row in reading order — what the arrow keys walk. */
export function flattenSections(sections: readonly PaletteSection[]): PaletteEntry[] {
  return sections.flatMap(section => section.entries);
}
