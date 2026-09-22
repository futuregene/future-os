import Prism from "prismjs";
import { CODE_LANGUAGE_BY_SUFFIX, codeLanguageForFile, codeTokenRows, highlightCode } from "../codeHighlight";
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
    ["style.scss", "scss"],
    ["server.conf", "ini"],
    ["run.bat", "batch"],
    ["Program.vb", "vbnet"],
  ])("maps %s to %s", (name, language) => {
    expect(codeLanguageForFile(name)).toBe(language);
  });

  test.each(["notes.txt", "server.log", "a.out", "Makefile", "boot.asm", "Widget.vue", "App.svelte", "report.xlsx", ".bashrc"])(
    "leaves non-source %s as plain text",
    name => expect(codeLanguageForFile(name)).toBeNull(),
  );

  test("ignores case and surrounding whitespace, and prefers the longest suffix", () => {
    expect(codeLanguageForFile("  MAIN.PY ")).toBe("python");
    expect(codeLanguageForFile("app.test.tsx")).toBe("tsx");
    expect(codeLanguageForFile("a.tar.gz.ts")).toBe("typescript");
  });

  test("every mapped suffix has a loaded grammar that colors its source", () => {
    for (const [suffix, language] of Object.entries(CODE_LANGUAGE_BY_SUFFIX)) {
      const tokens = highlightCode("const x = 1; # comment\ndef main(): pass\n", language);
      expect([suffix, language, tokens !== null]).toEqual([suffix, language, true]);
      expect(tokens!.map(token => token.text).join(""))
        .toBe("const x = 1; # comment\ndef main(): pass\n");
      expect(tokens!.some(token => token.color)).toBe(true);
    }
  });
});
