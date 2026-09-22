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
// Grammars for the file suffixes the phone previews as text (see
// `CODE_LANGUAGE_BY_SUFFIX`). Static imports for the same reason as above:
// Metro bundles only these grammars, with no DOM, WebView or network.
// Dependency order matters: a grammar that `extend`s another must load after
// it (`basic` → vbnet, markup-templating → php).
import "prismjs/components/prism-ruby";
import "prismjs/components/prism-perl";
import "prismjs/components/prism-lua";
import "prismjs/components/prism-r";
import "prismjs/components/prism-julia";
import "prismjs/components/prism-matlab";
import "prismjs/components/prism-fortran";
import "prismjs/components/prism-haskell";
import "prismjs/components/prism-elixir";
import "prismjs/components/prism-erlang";
import "prismjs/components/prism-clojure";
import "prismjs/components/prism-lisp";
import "prismjs/components/prism-scheme";
import "prismjs/components/prism-pascal";
import "prismjs/components/prism-nim";
import "prismjs/components/prism-zig";
import "prismjs/components/prism-dart";
import "prismjs/components/prism-scala";
import "prismjs/components/prism-groovy";
import "prismjs/components/prism-solidity";
import "prismjs/components/prism-basic";
import "prismjs/components/prism-vbnet";
import "prismjs/components/prism-less";
import "prismjs/components/prism-scss";
import "prismjs/components/prism-sass";
import "prismjs/components/prism-ini";
import "prismjs/components/prism-properties";
import "prismjs/components/prism-hcl";
import "prismjs/components/prism-batch";
import "prismjs/components/prism-markup-templating";
import "prismjs/components/prism-php";
import { codeColors } from "../theme/tokens";

/** Prism grammar per file suffix for in-app code previews. This mirrors the
 * phone's text route (`remote/fileTypes.ts`): every code-ish suffix the phone
 * can open in-app has a grammar here. Suffixes without one (`.asm` mixes NASM
 * and GAS dialects, and the bundled Prism build ships no grammar for the
 * `.vue` / `.svelte` single-file components) deliberately stay plain text.
 * Longest suffix wins, so `.d.ts` beats `.ts` and `.mjs` beats `.js`.
 *
 * Markdown and JSON are absent on purpose — they have richer preview kinds of
 * their own. Dialects share a grammar where Prism ships no dedicated one
 * (`.fish`/`.csh` → bash, MATLAB `.m` → matlab).
 */
export const CODE_LANGUAGE_BY_SUFFIX: Readonly<Record<string, string>> = {
  ".c": "c",
  ".h": "c",
  ".cc": "cpp",
  ".cpp": "cpp",
  ".cxx": "cpp",
  ".hh": "cpp",
  ".hpp": "cpp",
  ".hxx": "cpp",
  ".cs": "csharp",
  ".java": "java",
  ".kt": "kotlin",
  ".kts": "kotlin",
  ".scala": "scala",
  ".groovy": "groovy",
  ".gradle": "groovy",
  ".swift": "swift",
  ".go": "go",
  ".rs": "rust",
  ".dart": "dart",
  ".pas": "pascal",
  ".nim": "nim",
  ".zig": "zig",
  ".sol": "solidity",
  ".vb": "vbnet",

  ".py": "python",
  ".pyi": "python",
  ".rb": "ruby",
  ".php": "php",
  ".pl": "perl",
  ".pm": "perl",
  ".lua": "lua",
  ".r": "r",
  ".jl": "julia",
  ".m": "matlab",
  ".f90": "fortran",
  ".f95": "fortran",
  ".hs": "haskell",
  ".ex": "elixir",
  ".exs": "elixir",
  ".erl": "erlang",
  ".clj": "clojure",
  ".cljs": "clojure",
  ".lisp": "lisp",
  ".el": "lisp",
  ".scm": "scheme",

  ".js": "javascript",
  ".mjs": "javascript",
  ".cjs": "javascript",
  ".jsx": "jsx",
  ".ts": "typescript",
  ".d.ts": "typescript",
  ".tsx": "tsx",
  ".css": "css",
  ".less": "less",
  ".scss": "scss",
  ".sass": "sass",

  ".sh": "bash",
  ".bash": "bash",
  ".zsh": "bash",
  ".fish": "bash",
  ".csh": "bash",
  ".bat": "batch",
  ".cmd": "batch",
  ".ps1": "powershell",

  ".sql": "sql",
  ".ini": "ini",
  ".cfg": "ini",
  ".conf": "ini",
  ".env": "ini",
  ".properties": "properties",
  ".toml": "toml",
  ".tf": "hcl",
};

const CODE_LANGUAGE_SUFFIXES = Object.keys(CODE_LANGUAGE_BY_SUFFIX)
  .sort((left, right) => right.length - left.length);

/** Prism language for a previewed file name. Null means "read it as plain
 * text": not a code file at all (`.txt`, `.log`), or no grammar shipped for it.
 * `highlightCode` validates the name against the loaded grammars anyway, so an
 * unknown or misspelled entry degrades to plain text instead of throwing. */
export function codeLanguageForFile(name: string): string | null {
  const normalized = name.trim().toLowerCase();
  const suffix = CODE_LANGUAGE_SUFFIXES.find(candidate => normalized.endsWith(candidate));
  return suffix ? CODE_LANGUAGE_BY_SUFFIX[suffix]! : null;
}

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
  // Types Prism emits for the config / template languages the previews add
  // (CSS-family selectors, ini / .properties keys, batch commands, HCL types,
  // Ruby's string-literal, PHP's `<?php` delimiter). Without them those files
  // tokenize into spans that all render uncolored.
  selector: codeColors.keyword, section: codeColors.keyword, "section-name": codeColors.keyword,
  key: codeColors.property, value: codeColors.string, "string-literal": codeColors.string,
  command: codeColors.function, type: codeColors.keyword, delimiter: codeColors.operator,
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
