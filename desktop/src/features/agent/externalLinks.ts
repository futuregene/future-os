/**
 * Split text into verbatim and external `[label](https://…)` link segments for
 * user-message rendering. User messages are otherwise plain text (never
 * markdown — the user's `*`/`#`/`1.` stay literal); the only exception beyond
 * `@` file mentions (see `mentionMarkdown`) is a coach-prompt-style manual
 * link, which renders clickable via SafeLink. Only `http(s)` targets are
 * recognized, so stray brackets never turn into links.
 */
export interface ExternalLinkSegment {
  /** Verbatim text, or the link's display label. */
  text: string;
  /** True for a `[label](http…)` link segment; false for literal text. */
  link: boolean;
  /** The http(s) target, present only for link segments. */
  href?: string;
  /** Character offset in the source string — a stable, unique React key. */
  key: number;
}

const EXTERNAL_LINK = /\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g;

export function splitExternalLinkSegments(text: string): ExternalLinkSegment[] {
  const segments: ExternalLinkSegment[] = [];
  let last = 0;
  EXTERNAL_LINK.lastIndex = 0;
  for (let match = EXTERNAL_LINK.exec(text); match !== null; match = EXTERNAL_LINK.exec(text)) {
    if (match.index > last)
      segments.push({ text: text.slice(last, match.index), link: false, key: last });
    segments.push({ text: match[1] ?? "", link: true, href: match[2] ?? "", key: match.index });
    last = match.index + match[0].length;
  }
  if (last < text.length)
    segments.push({ text: text.slice(last), link: false, key: last });
  return segments;
}
