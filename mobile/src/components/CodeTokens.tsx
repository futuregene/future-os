import { Text } from "react-native";
import type { CodeToken } from "./codeHighlight";

/** One code block's colored spans. `tokens === null` — unknown language, an
 * oversized block, or a grammar failure — renders the untouched source, so
 * callers never need a second code path and highlighting can never alter the
 * text the reader selects or copies. */
export function CodeTokens({ tokens, fallback }: { tokens: CodeToken[] | null; fallback: string }) {
  if (!tokens) return fallback;
  return tokens.map((token, index) => token.color
    ? <Text key={index} style={{ color: token.color }}>{token.text}</Text>
    : token.text);
}
