/**
 * What a remote composer's `/` menu offers: the host's commands, and the skills
 * that host has installed.
 *
 * Two kinds because they are used in two ways, exactly as the local composer's
 * menu works: a command *runs* (compacting is a request to the host, not text)
 * while a skill is *named* — `/<name>`, which the host resolves when the prompt
 * arrives. Both are the host's own facts, so both come from it.
 *
 * Pure: what a draft's slash word matches, the one flat order the menu and the
 * keyboard share, and the name a chosen skill leaves in the draft. The menu that
 * draws them is `RemoteSlashMenu`, and this file is separate so that one file
 * exports one kind of thing (the fast-refresh rule enforces it, and it is right
 * to).
 */
export interface RemoteCommand {
  id: string;
  /** Localized label. */
  label: string;
  description: string;
  run: () => void;
}

/**
 * A skill the host has installed, as the `/` menu offers it.
 *
 * `name` is the name the *host* resolves — the one that goes into the draft —
 * and `description` is already in the reader's language. The other language's
 * description is kept for the search only: a Chinese user looking for "评审"
 * should find `/future-peer-review`, whose displayed description is Chinese but
 * whose name is not.
 */
export interface RemoteSkillChoice {
  name: string;
  description: string;
  descriptionZh: string | null;
}

export interface RemoteSlashMenuGroups {
  commands: RemoteCommand[];
  skills: RemoteSkillChoice[];
}

export type RemoteSlashItem
  = | { kind: "command"; command: RemoteCommand }
    | { kind: "skill"; skill: RemoteSkillChoice };

/**
 * The menu's one flat order: the commands that run, then the skills that are
 * named.
 *
 * Shared by the menu that draws the rows and the keyboard that walks them.
 * Computing it twice is how a highlight ends up on a row other than the one the
 * reader sees (the local composer keeps two copies of exactly this list in step
 * by hand), so there is one function and both callers use it.
 */
export function remoteSlashItems(groups: RemoteSlashMenuGroups): RemoteSlashItem[] {
  return [
    ...groups.commands.map(command => ({ kind: "command" as const, command })),
    ...groups.skills.map(skill => ({ kind: "skill" as const, skill })),
  ];
}

/**
 * The items a draft's slash word matches.
 *
 * The query is what follows the slash, so `/co` finds Compact — the same
 * substring rule the local menu uses, because a menu that matched differently
 * would make one `/` mean two things in two composers. Skills are capped at 20
 * as they are there: a menu nobody scrolls past 20 rows.
 */
export function buildRemoteSlashMenu(
  query: string,
  commands: RemoteCommand[],
  skills: RemoteSkillChoice[],
): RemoteSlashMenuGroups {
  const needle = query.toLowerCase();
  const matches = (values: Array<string | null | undefined>) =>
    values.some(value => (value ?? "").toLowerCase().includes(needle));
  return {
    commands: commands.filter(command =>
      !needle
      || command.id.toLowerCase().includes(needle)
      || command.label.toLowerCase().includes(needle)),
    skills: skills.filter(skill => matches([skill.name, skill.description, skill.descriptionZh])).slice(0, 20),
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

/**
 * The draft with its slash word replaced by the skill it stands for.
 *
 * The trailing space is the phone's: the skill is a word in a sentence, and the
 * next thing typed must not run into it (`/future-web给…` would name a skill
 * that does not exist).
 */
export function insertRemoteSkill(draft: string, name: string): string {
  return draft.replace(/(^|\s)\/\S*$/, `$1/${name} `);
}
