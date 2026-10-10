import Prism from "prismjs";
import {
  CODE_LANGUAGE_BY_NAME,
  CODE_LANGUAGE_BY_SUFFIX,
  codeLanguageForFile,
  codeStyleForFile,
  codeTokenRows,
  highlightCode,
} from "../codeHighlight";
import { codePreviewRows } from "../codePreviewRows";
import { codeColors } from "../../theme/tokens";

describe("native code highlighting", () => {
  test.each([
    ["tsx", '<TextInput underlineColorAndroid="transparent" />'],
    ["ts", 'const answer: string = "hello";'],
    ["jsx", '<div title="hello">text</div>'],
    ["js", "const answer = 42;"],
    ["python", 'def greet():\n  return "hello"'],
    ["rs", 'fn main() { println!("hello"); }'],
    ["json", '{"answer": true}'],
    ["sh", 'echo "$HOME"'],
    ["yml", "answer: true"],
    ["toml", 'name = "test"'],
    ["go", 'package main\nfunc main() {}'],
    ["java", "class Main {}"],
    ["c", "int main() { return 0; }"],
    ["c++", "const int count = 2;"],
    ["c#", "public class Main {}"],
    ["swift", 'let name = "test"'],
    ["kt", 'val name = "test"'],
    ["sql", "SELECT * FROM users;"],
    ["html", '<a href="test">link</a>'],
    ["css", "a { color: red; }"],
    ["md", "# heading"],
    ["diff", "+added\n-removed"],
    ["ps1", 'Write-Host "hello"'],
    ["dockerfile", "FROM node:22"],
  ])("highlights %s without altering source", (language, code) => {
    const tokens = highlightCode(code, language)!;
    expect(tokens).not.toBeNull();
    expect(tokens.map(token => token.text).join("")).toBe(code);
    expect(tokens.some(token => token.color)).toBe(true);
  });

  test("the screenshot's incomplete TSX attribute still colors its string", () => {
    expect(highlightCode('underlineColorAndroid="transparent"', " TSX "))
      .toContainEqual({ text: '"transparent"', color: codeColors.string });
  });

  test("keeps Unicode, entities, CRLF, indentation and trailing blank lines verbatim", () => {
    const code = '// 注释\r\nconst s = "<&中文🙂>";\r\n\r\n';
    expect(highlightCode(code, "js")!.map(token => token.text).join("")).toBe(code);
  });

  test.each([undefined, "", "text", "txt", "plain", "unknown", "mermaid", "constructor", "__proto__", "extend"])(
    "falls back to plain text for %s", language => {
      expect(highlightCode("const x = 1", language)).toBeNull();
    },
  );

  test("oversized code bypasses tokenization and excessive token counts fall back", () => {
    const tokenize = jest.spyOn(Prism, "tokenize");
    try {
      expect(highlightCode("x".repeat(30_001), "ts")).toBeNull();
      expect(tokenize).not.toHaveBeenCalled();
      expect(highlightCode("x=1;".repeat(2000), "js")).toBeNull();
    } finally { tokenize.mockRestore(); }
  });

  test("a grammar failure never takes down the message", () => {
    const tokenize = jest.spyOn(Prism, "tokenize").mockImplementation(() => { throw new Error("grammar"); });
    try { expect(highlightCode("x", "ts")).toBeNull(); }
    finally { tokenize.mockRestore(); }
  });

  test("virtual rows keep multiline comment context across chunk boundaries", () => {
    const code = "/*\n" + "comment line\n".repeat(70) + "*/\nconst x = 1;\n";
    const rows = codePreviewRows(code);
    const result = codeTokenRows(highlightCode(code, "ts"), rows);
    expect(rows.length).toBeGreaterThan(2);
    for (const [index, row] of rows.entries()) {
      expect(result[index]!.map(token => token.text).join(""))
        .toBe(row.text.endsWith("\n") ? row.text.slice(0, -1) : row.text);
    }
    expect(result[1]!.every(token => token.color === codeColors.comment)).toBe(true);
    expect(codeTokenRows(null, rows)).toEqual(rows.map(() => null));
  });
});

describe("previewed file names pick a grammar", () => {
  test.each([
    ["main.py", "python"],
    ["lib.rs", "rust"],
    ["App.tsx", "tsx"],
    ["types.d.ts", "typescript"],
    ["index.mjs", "javascript"],
    ["index.mts", "typescript"],
    ["main.go", "go"],
    ["deploy.sh", "bash"],
    ["setup.ps1", "powershell"],
    ["schema.sql", "sql"],
    ["Cargo.toml", "toml"],
    ["analysis.R", "r"],
    ["labels.scm", "scheme"],
    ["init.el", "lisp"],
    ["build.gradle", "groovy"],
    ["main.tf", "hcl"],
    ["prod.tfvars", "hcl"],
    ["style.scss", "scss"],
    ["server.conf", "ini"],
    ["run.bat", "batch"],
    ["Program.vb", "vbnet"],
    ["values.yaml", "yaml"],
    ["layout.xml", "xml"],
    ["page.xhtml", "html"],
    ["rows.ndjson", "json"],
    ["api.proto", "protobuf"],
    ["schema.graphql", "graphql"],
    ["Main.elm", "elm"],
    ["AppDelegate.mm", "objectivec"],
    ["legacy.f", "fortran"],
    ["review.diff", "diff"],
    ["changes.patch", "diff"],
    ["build.mk", "makefile"],
    // Suffix-less files match on the whole name.
    ["Makefile", "makefile"],
    ["GNUmakefile", "makefile"],
    ["Dockerfile", "docker"],
    ["Dockerfile.dev", "docker"],
    ["Jenkinsfile", "groovy"],
    ["Gemfile", "ruby"],
    ["Podfile", "ruby"],
    [".gitignore", "git"],
    [".bashrc", "bash"],
    [".npmrc", "ini"],
    [".env.local", "ini"],
  ])("maps %s to %s", (name, language) => {
    expect(codeLanguageForFile(name)).toBe(language);
  });

  test.each(["notes.txt", "server.log", "a.out", "boot.asm", "Widget.vue", "App.svelte", "report.xlsx", "Cargo.lock", "go.mod", "meson.build"])(
    "leaves non-source %s as plain text",
    name => expect(codeLanguageForFile(name)).toBeNull(),
  );

  test("ignores case and surrounding whitespace, and prefers the longest suffix", () => {
    expect(codeLanguageForFile("  MAIN.PY ")).toBe("python");
    expect(codeLanguageForFile("app.test.tsx")).toBe("tsx");
    expect(codeLanguageForFile("a.tar.gz.ts")).toBe("typescript");
    expect(codeLanguageForFile("C:\\w\\MAKEFILE")).toBe("makefile");
    // Only the base name decides: a directory part is not the file's type.
    expect(codeLanguageForFile("/w/notes.md/scratch")).toBeNull();
  });

  test("lays code, config and data out monospace and prose proportional", () => {
    for (const name of ["main.py", "Makefile", "Cargo.lock", "table.csv", "meson.build", "boot.asm", "Widget.vue", "go.mod"])
      expect([name, codeStyleForFile(name)]).toEqual([name, true]);
    for (const name of ["notes.txt", "server.log", "LICENSE", "README"])
      expect([name, codeStyleForFile(name)]).toEqual([name, false]);
  });

  test("every mapped suffix and name has a loaded grammar that colors its source", () => {
    // The sample carries one trigger for each family: a line-leading `#` (git's
    // ignore-file comments, makefile recipes), an added/removed diff line, a
    // tag (markup), a number and a `def`. A grammar that colors nothing at all
    // would render as plain text and no screenshot would flag it.
    const sample = "# comment\n+added\n-removed\n<x>text</x>\nconst x = 1;\ndef main(): pass\n";
    for (const [key, language] of [
      ...Object.entries(CODE_LANGUAGE_BY_SUFFIX),
      ...Object.entries(CODE_LANGUAGE_BY_NAME),
    ]) {
      const tokens = highlightCode(sample, language);
      expect([key, language, tokens !== null]).toEqual([key, language, true]);
      expect(tokens!.map(token => token.text).join("")).toBe(sample);
      expect(tokens!.some(token => token.color)).toBe(true);
    }
  });
});
