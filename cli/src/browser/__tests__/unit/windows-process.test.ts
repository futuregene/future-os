import { describe, expect, test } from "bun:test";
import {
  buildStartProcessScript,
  quoteWindowsCommandLineArgument,
} from "../../windows-process.js";

describe("Windows detached process launcher", () => {
  test("quotes argv values using Windows command-line rules", () => {
    expect(quoteWindowsCommandLineArgument("--no-first-run")).toBe("--no-first-run");
    expect(quoteWindowsCommandLineArgument("--user-data-dir=C:\\Users\\Ace User\\profile"))
      .toBe('"--user-data-dir=C:\\Users\\Ace User\\profile"');
    expect(quoteWindowsCommandLineArgument('a"b')).toBe('"a\\"b"');
  });

  test("builds an encoded-command-safe Start-Process script", () => {
    const script = buildStartProcessScript(
      "C:\\Program Files\\Browser's App\\chrome.exe",
      ["--flag", "value with spaces"],
    );

    expect(script).toContain("Start-Process -FilePath 'C:\\Program Files\\Browser''s App\\chrome.exe'");
    expect(script).toContain("-ArgumentList '--flag \"value with spaces\"'");
    expect(script).toContain("-WindowStyle Normal -PassThru");
    expect(script).not.toContain("cmd /c");
  });
});
