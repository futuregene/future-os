/**
 * How the demo reply quotes the request it is answering.
 *
 * A demo conversation's answer opens by naming what it was asked, so the quote
 * has to be the text the reader actually sent. It used to be the first 16
 * characters (`prompt.slice(0, 16)`), which cut a 19-character request at
 * "…综述初稿并" — a half sentence nobody wrote, and it read as if part of the
 * message had been lost on the way to the model (#1534). The pipeline is fine:
 * the composer sends the whole text and the Agent receives it; only the demo's
 * reply truncated what it echoed.
 *
 * So the whole prompt is quoted. A prompt too long to repeat in full is
 * abbreviated with a trailing ellipsis — the mark a person leaves when they
 * deliberately shorten a long request, rather than a sentence that appears to
 * stop mid-way.
 */
export const QUOTED_PROMPT_LIMIT = 60;

export function quotedPrompt(prompt: string): string {
  const text = prompt.trim();
  // Count code points, not UTF-16 units: slicing the string by index can cut a
  // surrogate pair in half and leave a replacement character at the ellipsis.
  const characters = Array.from(text);
  if (characters.length <= QUOTED_PROMPT_LIMIT)
    return text;
  return `${characters.slice(0, QUOTED_PROMPT_LIMIT).join("")}…`;
}
