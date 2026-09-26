import { describe, expect, it } from "vitest";
import { localPathsFromUriList } from "./clipboardAttachments";

describe("localPathsFromUriList", () => {
  it("decodes local paths, Windows drive paths, and UNC paths", () => {
    expect(localPathsFromUriList("# comment\nfile:///Users/me/a%20b.txt\nfile:///C:/work/a.txt\nfile://server/share/a.txt")).toEqual([
      "/Users/me/a b.txt",
      "C:/work/a.txt",
      "\\\\server\\share\\a.txt",
    ]);
  });

  it("ignores external URIs and duplicate local paths", () => {
    expect(localPathsFromUriList("https://example.com/a\nfile:///tmp/a\nfile:///tmp/a")).toEqual(["/tmp/a"]);
  });

  it("drops a file URI it cannot turn into a path", () => {
    // boundary: a `file:` URI that decodes to an empty path (or an undecodable
    // percent escape) is not a usable local path, so it must be skipped rather than
    // forwarded as an empty string for the attach step to trip over.
    expect(localPathsFromUriList("file://%\nfile:///ok.txt")).toEqual(["/ok.txt"]);
    // A bare `file:` scheme carries no path at all.
    expect(localPathsFromUriList("file:")).toEqual([]);
    // Whitespace-only and comment-only input yields nothing.
    expect(localPathsFromUriList("  \n# gone\n")).toEqual([]);
  });
});
