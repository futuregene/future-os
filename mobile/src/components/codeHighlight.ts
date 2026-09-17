import { languages, tokenize } from "prismjs";
import type { Token } from "prismjs";
// Static imports let Metro bundle only these grammars, without a DOM, WebView,
// dynamic require, or downloading code. Dependency order matters for JSX/TSX.
import "prismjs/components/prism-typescript";
import "prismjs/components/prism-jsx";
import "prismjs/components/prism-tsx";
import "prismjs/components/prism-python";
import "prismjs/components/prism-rust";
import "prismjs/components/prism-json";
import "prismjs/components/prism-bash";
import "prismjs/components/prism-yaml";
import "prismjs/components/prism-toml";
import "prismjs/components/prism-go";
import "prismjs/components/prism-java";
import "prismjs/components/prism-c";
import "prismjs/components/prism-cpp";
import "prismjs/components/prism-csharp";
import "prismjs/components/prism-swift";
import "prismjs/components/prism-kotlin";
import "prismjs/components/prism-sql";
import "prismjs/components/prism-markdown";
import "prismjs/components/prism-diff";
import "prismjs/components/prism-powershell";
import "prismjs/components/prism-docker";
import { codeColors } from "../theme/tokens";

export interface CodeToken { text: string; color?: string }

const aliases: Record<string, string> = {
  shell: "bash", shellscript: "bash", zsh: "bash",
  rs: "rust", golang: "go", kt: "kotlin", "c++": "cpp", "c#": "csharp",
  dockerfile: "docker", ps1: "powershell", text: "plain", txt: "plain",
};
const tokenColors: Record<string, string> = {
  comment: codeColors.comment, prolog: codeColors.comment, doctype: codeColors.comment,
  keyword: codeColors.keyword, builtin: codeColors.keyword, tag: codeColors.keyword,
  boolean: codeColors.literal, number: codeColors.literal, constant: codeColors.literal,
  string: codeColors.string, char: codeColors.string, regex: codeColors.string,
  "attr-value": codeColors.string, inserted: codeColors.string,
  function: codeColors.function, "class-name": codeColors.function,
  property: codeColors.property, "attr-name": codeColors.property, variable: codeColors.property,
  operator: codeColors.operator, punctuation: codeColors.operator,
  deleted: codeColors.keyword,
};

/** Whole-block tokenization preserves multiline strings/comments. Bound both
 * regex input and native Text spans; oversized/unknown code stays selectable.
 * Never emit HTML: entities, indentation and source text remain untouched. */
export function highlightCode(code: string, language?: string): CodeToken[] | null {
  if (!language || code.length > 30_000) return null;
  const name = language.trim().toLowerCase();
  const normalized = Object.hasOwn(aliases, name) ? aliases[name]! : name;
  if (!Object.hasOwn(languages, normalized)) return null;
  const grammar = languages[normalized];
  if (!grammar || typeof grammar !== "object" || Object.keys(grammar).length === 0) return null;
  try {
    const result: CodeToken[] = [];
    function append(value: string | Token | (string | Token)[], color?: string) {
      if (typeof value === "string") {
        const last = result[result.length - 1];
        if (last && last.color === color) last.text += value;
        else if (value) result.push({ text: value, color });
        if (result.length > 6000) throw new Error("Code token budget exceeded");
      } else if (Array.isArray(value)) {
        value.forEach(part => append(part, color));
      } else {
        const types = [value.type, ...[value.alias ?? []].flat()];
        append(value.content, types.map(type => Object.hasOwn(tokenColors, type) ? tokenColors[type] : undefined)
          .find(candidate => candidate !== undefined) ?? color);
      }
    }
    append(tokenize(code, grammar));
    return result;
  } catch {
    return null;
  }
}

/** Partition a highlighted block using the existing virtualized source chunks,
 * without re-tokenizing each chunk (which would lose multiline grammar state). */
export function codeTokenRows(tokens: CodeToken[] | null, rows: { text: string }[]): (CodeToken[] | null)[] {
  if (!tokens) return rows.map(() => null);
  let index = 0;
  let offset = 0;
  return rows.map(row => {
    const result: CodeToken[] = [];
    let remaining = row.text.length;
    while (remaining > 0 && index < tokens.length) {
      const token = tokens[index]!;
      const length = Math.min(remaining, token.text.length - offset);
      result.push({ text: token.text.slice(offset, offset + length), color: token.color });
      remaining -= length;
      offset += length;
      if (offset === token.text.length) { index++; offset = 0; }
    }
    // The row Text supplies its own line ending; preserve source in row data.
    const last = result[result.length - 1];
    if (last?.text.endsWith("\n")) last.text = last.text.slice(0, -1);
    return result;
  });
}
