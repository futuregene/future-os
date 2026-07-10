/**
 * Shared parser for the composer's `@`-mention markdown. A mention serializes to
 * `[name](./path)` — or `[name](<./path>)` when the path holds spaces/parens.
 * The same string is what gets sent to the model, what MessageBlock renders, and
 * what a composer draft stores, so both the renderer and the editor restore path
 * split it here rather than duplicating the regex.
 */
export interface MentionSegment {
  /** Mention: the display name. Plain segment: literal text (verbatim). */
  text: string;
  /** True when this segment is a file mention (renders/rebuilds as a pill). */
  mention: boolean;
  /** The `./path` target, present only for mention segments. */
  path?: string;
  /** Character offset in the source string — a stable, unique React key. */
  key: number;
}

const MENTION_LINK = /\[([^\]]+)\]\((?:<(\.\/[^>]+)>|(\.\/[^)\s]+))\)/g;

/** Split content into verbatim-text and file-mention segments, in order. */
export function parseMentionSegments(content: string): MentionSegment[] {
  const segments: MentionSegment[] = [];
  let last = 0;
  MENTION_LINK.lastIndex = 0;
  for (let match = MENTION_LINK.exec(content); match; match = MENTION_LINK.exec(content)) {
    if (match.index > last)
      segments.push({ text: content.slice(last, match.index), mention: false, key: last });
    segments.push({ text: match[1] ?? "", mention: true, path: match[2] ?? match[3] ?? "", key: match.index });
    last = match.index + match[0].length;
  }
  if (last < content.length)
    segments.push({ text: content.slice(last), mention: false, key: last });
  return segments;
}
