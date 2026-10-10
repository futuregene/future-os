import { findSessionReferences } from "@future-os/markdown";

/**
 * Shared parser for the composer's reference markdown. A file mention serializes
 * to `[name](./path)` — or `[name](<./path>)` when the path holds spaces/parens;
 * a session reference serializes to `[title](futureos://session/<id>)` (see
 * `@future-os/markdown`). Both strings are what gets sent to the model, what
 * MessageBlock renders, and what a composer draft stores, so the renderer and
 * the editor restore path split them here rather than duplicating the regex.
 */
export interface MentionSegment {
  /** Reference: the display name/title. Plain segment: literal text (verbatim). */
  text: string;
  /** True when this segment is a reference (renders/rebuilds as a pill). */
  mention: boolean;
  /** The `./path` target, present only for file mentions. */
  path?: string;
  /** The agent session id, present only for session references. */
  sessionId?: string;
  /** Character offset in the source string — a stable, unique React key. */
  key: number;
}

const MENTION_LINK = /\[([^\]]+)\]\((?:<(\.\/[^>]+)>|(\.\/[^)\s]+))\)/g;

/** Split content into verbatim-text and reference segments, in order. */
export function parseMentionSegments(content: string): MentionSegment[] {
  // Both reference kinds are collected and then emitted in source order, so a
  // session reference before a file mention keeps its position.
  const hits: Array<{
    index: number;
    length: number;
    text: string;
    path?: string;
    sessionId?: string;
  }> = [];
  MENTION_LINK.lastIndex = 0;
  for (let match = MENTION_LINK.exec(content); match; match = MENTION_LINK.exec(content)) {
    hits.push({
      index: match.index,
      length: match[0].length,
      text: match[1] ?? "",
      path: match[2] ?? match[3] ?? "",
    });
  }
  for (const reference of findSessionReferences(content)) {
    hits.push({
      index: reference.index,
      length: reference.length,
      text: reference.title,
      sessionId: reference.sessionId,
    });
  }
  hits.sort((left, right) => left.index - right.index);

  const segments: MentionSegment[] = [];
  let last = 0;
  for (const hit of hits) {
    // The two patterns cannot overlap; a hit inside one already emitted would
    // only come from a malformed line, and must not split text twice.
    if (hit.index < last)
      continue;
    if (hit.index > last)
      segments.push({ text: content.slice(last, hit.index), mention: false, key: last });
    segments.push(hit.sessionId
      ? { text: hit.text, mention: true, sessionId: hit.sessionId, key: hit.index }
      : { text: hit.text, mention: true, path: hit.path, key: hit.index });
    last = hit.index + hit.length;
  }
  if (last < content.length)
    segments.push({ text: content.slice(last), mention: false, key: last });
  return segments;
}
