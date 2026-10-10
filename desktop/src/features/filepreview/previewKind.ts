import { pathBasename, pathExtension } from "../../lib/workspacePath";

/**
 * File-type detection for the local-file preview overlay. Detection is purely by
 * extension — the same signal `FileLink` has (a `StoredFile` never touches the
 * filesystem). Only these kinds get an in-app preview; every other file (PDFs
 * included) keeps the "open with the OS default handler" behavior.
 */
export type PreviewKind = "image" | "json" | "markdown" | "text";

const IMAGE_RE = /\.(?:avif|bmp|gif|jpe?g|png|svg|webp)$/i;
const MARKDOWN_RE = /\.(?:md|markdown)$/i;
// A notebook is one JSON document, so the JSON reader can show it.
const JSON_RE = /\.(?:ipynb|json)$/i;

/**
 * Code / config / script files read as plain monospace text. Deliberately a
 * separate list from `lib/fileType`'s `CODE_RE`: that one is a *glyph* signal
 * (which icon a row shows) and classifies some types this overlay can't read,
 * while this is the *capability* signal. It is also wider than the phone's
 * text routes (`mobile/src/remote/fileTypes.ts`) — the overlay has no transfer
 * budget and reads whatever the OS can hand it.
 */
const TEXT_RE
  = /\.(?:[cfhmr]|asm|bash|bat|cc|cfg|clj|cljs|cmake|cmd|conf|cpp|cs|csh|csproj|css|csv|cts|cxx|dart|diff|edn|el|elm|env|erl|ex|exs|f90|f95|fish|go|gradle|graphql|groovy|hcl|hh|hpp|hs|htm|html|hxx|ini|java|jl|js|jsonl|jsx|kt|kts|less|lisp|lock|lua|mk|mm|mts|ndjson|nim|pas|patch|php|pl|plist|pm|properties|proto|ps1|py|pyi|rb|rs|sass|scala|scm|scss|sh|sln|sol|sql|svelte|swift|tf|tfvars|toml|ts|tsv|tsx|vb|vue|xcconfig|xhtml|xml|yaml|yml|zig|zsh)$/i;

/**
 * Suffix-less files the overlay reads as text — build entry points, lockfiles,
 * dependency manifests, editor / shell dotfiles. A suffix table can never see
 * them, and the phone's own list (`mobile/src/remote/fileTypes.ts`,
 * `MOBILE_FILE_NAMES`) carries the same names, minus the extra ones the
 * overlay can afford.
 */
const TEXT_NAMES = new Set([
  ".babelrc",
  ".bashrc",
  ".bazelrc",
  ".condarc",
  ".dockerignore",
  ".editorconfig",
  ".envrc",
  ".eslintrc",
  ".gitattributes",
  ".gitignore",
  ".gitmodules",
  ".npmrc",
  ".nvmrc",
  ".prettierrc",
  ".profile",
  ".rprofile",
  ".vimrc",
  ".yamllint",
  ".zshrc",
  "authors",
  "brewfile",
  "build.bazel",
  "caddyfile",
  "changelog",
  "codeowners",
  "contributing",
  "copying",
  "gemfile",
  "go.mod",
  "go.sum",
  "justfile",
  "license",
  "meson.build",
  "notice",
  "podfile",
  "procfile",
  "rakefile",
  "readme",
  "vagrantfile",
  "workspace",
]);

/**
 * Names that qualify themselves with a suffix (`Dockerfile.dev`, `Makefile.am`,
 * `.env.local`): the prefix alone, or the prefix followed by `.`.
 */
const TEXT_NAME_PREFIXES = ["makefile", "gnumakefile", "dockerfile", "containerfile", "jenkinsfile", ".env"];

function isTextName(path: string): boolean {
  const base = pathBasename(path).toLowerCase();
  if (TEXT_NAMES.has(base))
    return true;
  return TEXT_NAME_PREFIXES.some(prefix => base === prefix || base.startsWith(`${prefix}.`));
}

export function previewKindForPath(path: string): PreviewKind | null {
  if (IMAGE_RE.test(path))
    return "image";
  if (MARKDOWN_RE.test(path))
    return "markdown";
  if (JSON_RE.test(path))
    return "json";
  if (TEXT_RE.test(path) || isTextName(path))
    return "text";
  return null;
}

/**
 * True for paths whose bytes the text-preview backend command can render —
 * plain code / config text, plus the Markdown and JSON files that have a richer
 * preview kind of their own. Callers that just want "is this readable text"
 * (the artifact detail's file-backed preview) use this instead of switching on
 * {@link PreviewKind}.
 */
export function isTextReadablePath(path: string): boolean {
  return MARKDOWN_RE.test(path) || JSON_RE.test(path) || TEXT_RE.test(path) || isTextName(path);
}

/**
 * Syntax grammar per file extension for the text preview. Values are Shiki
 * language ids/aliases (`useCodeHighlighter` validates them against Shiki's
 * registry, so an unknown id degrades to plain text instead of throwing — and
 * the preview tests assert every value really is in that registry).
 *
 * Reason for this table to be plain strings rather than a Shiki import: this
 * module is imported by the file tree, artifact panel and message links — none
 * of which should pull the highlighter into their bundle just to classify a
 * path. Coverage is deliberately a superset of {@link TEXT_RE} where Shiki has
 * a grammar, plus JSON and TSV-style data files the artifact panel renders in
 * the same text `<pre>`, minus the files that have a richer preview kind of
 * their own (Markdown, images). Extensions with no grammar stay unlisted and
 * plain.
 *
 * Tie-breaks, where one extension is shared by two languages: `.m` follows the
 * phone's choice (MATLAB), `.csh` renders as shell, `.el` is Emacs Lisp, `.pl`
 * / `.pm` Perl. `.f` is fixed-form Fortran, `.f90` / `.f95` free-form.
 */
export const LANGUAGE_BY_EXTENSION: Record<string, string> = {
  c: "c",
  cc: "cpp",
  cpp: "cpp",
  cxx: "cpp",
  h: "c",
  hh: "cpp",
  hpp: "cpp",
  hxx: "cpp",
  cs: "csharp",
  java: "java",
  kt: "kotlin",
  kts: "kotlin",
  scala: "scala",
  groovy: "groovy",
  gradle: "groovy",
  swift: "swift",
  go: "go",
  rs: "rust",
  dart: "dart",
  pas: "pascal",
  nim: "nim",
  zig: "zig",
  sol: "solidity",
  vb: "vb",
  m: "matlab",
  mm: "objective-cpp",

  py: "python",
  pyi: "python",
  rb: "ruby",
  php: "php",
  pl: "perl",
  pm: "perl",
  lua: "lua",
  r: "r",
  jl: "julia",
  f: "fortran-fixed-form",
  f90: "fortran-free-form",
  f95: "fortran-free-form",
  hs: "haskell",
  ex: "elixir",
  exs: "elixir",
  erl: "erlang",
  clj: "clojure",
  cljs: "clojure",
  edn: "clojure",
  lisp: "common-lisp",
  el: "emacs-lisp",
  scm: "scheme",
  asm: "asm",

  js: "javascript",
  jsx: "jsx",
  ts: "typescript",
  tsx: "tsx",
  vue: "vue",
  svelte: "svelte",
  css: "css",
  less: "less",
  scss: "scss",
  sass: "sass",
  html: "html",
  htm: "html",
  xhtml: "html",
  xml: "xml",

  sh: "shellscript",
  bash: "shellscript",
  zsh: "shellscript",
  csh: "shellscript",
  fish: "fish",
  bat: "bat",
  cmd: "bat",
  ps1: "powershell",

  sql: "sql",
  json: "json",
  ipynb: "json",
  jsonl: "jsonl",
  ndjson: "jsonl",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  tf: "hcl",
  hcl: "hcl",
  tfvars: "hcl",
  ini: "ini",
  cfg: "ini",
  conf: "ini",
  env: "ini",
  properties: "ini",
  csv: "csv",
  tsv: "tsv",
  graphql: "graphql",
  elm: "elm",
  diff: "diff",
  patch: "diff",
  mk: "make",
  cmake: "cmake",
  proto: "protobuf",
  mts: "typescript",
  cts: "typescript",
  plist: "xml",
  csproj: "xml",
};

/**
 * Shiki language per bare file name, for the suffix-less files
 * {@link isTextName} accepts and the two whose extension hides their language
 * (`CMakeLists.txt`, `.env.local`). Same contract as
 * {@link LANGUAGE_BY_EXTENSION}: an id Shiki does not ship stays unlisted (the
 * preview then shows plain monospace text), and a prefix entry also matches
 * `name.…`.
 */
export const LANGUAGE_BY_NAME: Record<string, string> = {
  "makefile": "make",
  "gnumakefile": "make",
  "dockerfile": "dockerfile",
  "containerfile": "dockerfile",
  "jenkinsfile": "groovy",
  "justfile": "just",
  "gemfile": "ruby",
  "rakefile": "ruby",
  "podfile": "ruby",
  "vagrantfile": "ruby",
  "brewfile": "ruby",
  "cmakelists.txt": "cmake",
  ".env": "ini",
  ".bashrc": "shellscript",
  ".zshrc": "shellscript",
  ".profile": "shellscript",
  ".envrc": "shellscript",
  ".npmrc": "ini",
  ".editorconfig": "ini",
  ".condarc": "yaml",
  ".yamllint": "yaml",
};

const LANGUAGE_NAME_PREFIXES = ["makefile", "gnumakefile", "dockerfile", "containerfile", "jenkinsfile", ".env"];

/**
 * Shiki language for a path, or null when it has no syntax grammar — the
 * caller then shows the file as plain monospace text.
 */
export function codeLanguageForPath(path: string): string | null {
  const base = pathBasename(path).toLowerCase();
  const named = LANGUAGE_BY_NAME[base];
  if (named)
    return named;
  const prefix = LANGUAGE_NAME_PREFIXES.find(candidate => base.startsWith(`${candidate}.`));
  if (prefix)
    return LANGUAGE_BY_NAME[prefix]!;
  return LANGUAGE_BY_EXTENSION[pathExtension(path)] ?? null;
}

const IMAGE_MIME: Record<string, string> = {
  avif: "image/avif",
  bmp: "image/bmp",
  gif: "image/gif",
  jpeg: "image/jpeg",
  jpg: "image/jpeg",
  png: "image/png",
  svg: "image/svg+xml",
  webp: "image/webp",
};

/** MIME type for a data-URL `<img src>`, keyed off the extension. */
export function imageMimeForPath(path: string): string {
  return IMAGE_MIME[pathExtension(path)] ?? "application/octet-stream";
}
