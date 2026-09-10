import type { AgentMessage } from "@future-os/thread-projection";
import { parseFutureMarkdown } from "../markdown/parseFutureMarkdown";

// A conservative prefilter, not the final result list. The DOM remains the
// authority for visible Markdown and precise text ranges. Dynamic tool/status
// labels cannot be resolved from message text alone, so materialize those rows
// rather than risk excluding a match.
export function mayContainThreadSearch(message: AgentMessage, query: string): boolean {
  if (!query)
    return false;
  if (message.activityItems?.length || message.attachments?.length
    || message.segments?.some(segment => segment.kind !== "text")
    || message.status === "failed" || message.terminationNotice) {
    return true;
  }
  const text = message.segments?.length
    ? message.segments.map(segment => segment.kind === "text" ? segment.text : "").join("")
    : message.content;
  const needle = query.toLocaleLowerCase();
  if (text.toLocaleLowerCase().includes(needle))
    return true;
  if (message.role === "user")
    return false;
  const document = parseFutureMarkdown(text);
  if (document.references.length)
    return true;
  // Flatten the same parsed nodes used by MarkdownContent, so formatting such
  // as "a **formatted** phrase" does not hide an otherwise contiguous match.
  return textLeaves(document.nodes).toLocaleLowerCase().includes(needle);
}

function textLeaves(value: unknown): string {
  if (Array.isArray(value))
    return value.map(textLeaves).join("");
  if (!value || typeof value !== "object")
    return "";
  const node = value as Record<string, unknown>;
  if (typeof node.text === "string")
    return node.text;
  if (typeof node.code === "string")
    return node.code;
  return ["children", "items", "blocks", "headers", "rows"].map(key => textLeaves(node[key])).join("");
}
