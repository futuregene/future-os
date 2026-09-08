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
});
