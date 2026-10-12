/**
 * The commands a remote composer accepts after `/`.
 *
 * Pure: what a draft's slash word matches, and the label an item carries. The
 * menu that draws them is `RemoteSlashMenu`, and this file is separate so that
 * one file exports one kind of thing (the fast-refresh rule enforces it, and it
 * is right to).
 */
export interface RemoteCommand {
  id: string;
  /** Localized label. */
  label: string;
  description: string;
  run: () => void;
}

export interface RemoteSlashMenuGroups {
  commands: RemoteCommand[];
}

/**
 * The commands a draft's slash word matches.
 *
 * The query is what follows the slash, so `/co` finds Compact — the same
 * substring rule the local menu uses, because a menu that matched differently
 * would make one `/` mean two things in two composers.
 */
export function buildRemoteSlashMenu(query: string, commands: RemoteCommand[]): RemoteSlashMenuGroups {
  const needle = query.toLowerCase();
  return {
    commands: commands.filter(command =>
      !needle
      || command.id.toLowerCase().includes(needle)
      || command.label.toLowerCase().includes(needle)),
  };
}

/**
 * The slash word a draft is currently typing, or null.
 *
 * Only a slash that begins the draft (after whitespace) counts: a `/` inside a
 * sentence — a path, a fraction — must not open a menu over the message being
 * written.
 */
export function slashQuery(draft: string): string | null {
  const match = /(?:^|\s)\/(\S*)$/.exec(draft);
  return match ? match[1]! : null;
}

/** The draft without its slash word, for when a command is chosen. */
export function withoutSlashWord(draft: string): string {
  return draft.replace(/(^|\s)\/\S*$/, "$1").trimEnd();
}
