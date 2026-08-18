/**
 * Split user-message text into verbatim text, `[name](./path)` file mentions
 * and `[label](https://…)` links — the same recognition as the desktop
 * UserMessageText (mentionMarkdown + externalLinks regexes), so a prompt like
 * `[poem.txt](./poem.txt) 里面内容是什么` renders the label instead of the raw
 * markdown. Everything else stays literal (the user's `*`/`#`/`1.` never turn
 * into markdown).
 */
export interface UserTextSegment {
  text: string;
  kind: "plain" | "mention" | "link";
  /** Mention: the `./path` target with a single leading `./` stripped (same
   *  normalization as the shared markdown `localFilePath`). Link: the http(s)
   *  URL. Absent for plain segments. */
  href?: string;
  /** Character offset in the source string — a stable, unique React key. */
  key: number;
}

const MENTION_LINK = /\[([^\]]+)\]\((?:<(\.\/[^>]+)>|(\.\/[^)\s]+))\)/g;
const EXTERNAL_LINK = /\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g;

export function splitUserTextSegments(text: string): UserTextSegment[] {
  const segments: UserTextSegment[] = [];
  let last = 0;
  while (last < text.length) {
    MENTION_LINK.lastIndex = last;
    EXTERNAL_LINK.lastIndex = last;
    const mention = MENTION_LINK.exec(text);
    const external = EXTERNAL_LINK.exec(text);
    const next =
      mention && (!external || mention.index <= external.index)
        ? {
            index: mention.index,
            end: mention.index + mention[0].length,
            kind: "mention" as const,
            label: mention[1] ?? "",
            href: (mention[2] ?? mention[3] ?? "").replace(/^\.\//, ""),
          }
        : external
          ? {
              index: external.index,
              end: external.index + external[0].length,
              kind: "link" as const,
              label: external[1] ?? "",
              href: external[2] ?? "",
            }
          : null;
    if (!next) {
      segments.push({ text: text.slice(last), kind: "plain", key: last });
      return segments;
    }
    if (next.index > last) {
      segments.push({ text: text.slice(last, next.index), kind: "plain", key: last });
    }
    segments.push({ text: next.label, kind: next.kind, href: next.href, key: next.index });
    last = next.end;
  }
  return segments;
}
