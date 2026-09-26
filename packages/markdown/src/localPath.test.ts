import { describe, expect, it } from "vitest";
import {
  basename,
  classifyMarkdownTarget,
  localFilePath,
  remoteMarkdownImageUrl,
} from "./localPath";

describe("localFilePath", () => {
  it.each([
    // Rejected: no path-ish signal at all.
    ["", null],
    ["   ", null],
    // Explicit schemes are never local. A one-letter prefix (`C:`) is not a
    // scheme, so it falls through to the drive handling below.
    ["http://example.com/a.md", null],
    ["https://example.com", null],
    ["mailto:someone@example.com", null],
    ["futureos://file/abc", null],
    ["C:", null],
    // Scheme-relative URLs stay blocked instead of becoming a POSIX path.
    ["//cdn.example.com/x.md", null],
    // Bare web hosts stay remote even when they carry a path or port.
    ["example.com", null],
    ["example.com/page", null],
    ["github.com/user/repo", null],
    ["example.com:8080/page", null],
    // No extension and no separator → ambiguous, stays remote.
    ["README", null],
    [".bashrc", null],
    ["a.b/c", "a.b/c"],
  ])("classifies %j as %j", (href, expected) => {
    expect(localFilePath(href)).toBe(expected);
  });

  it.each([
    ["/Users/x/notes.md", "/Users/x/notes.md"],
    ["C:/work/a.ts", "C:/work/a.ts"],
    ["c:\\work\\a.ts", "c:\\work\\a.ts"],
    ["\\\\server\\share\\a.md", "\\\\server\\share\\a.md"],
    ["./docs/readme.md", "docs/readme.md"],
    ["../up.md", "../up.md"],
    [".\\win.md", ".\\win.md"],
    ["\\rooted.md", "\\rooted.md"],
    ["docs/readme.md", "docs/readme.md"],
    ["长诗.md", "长诗.md"],
    ["config.json", "config.json"],
    ["  spaced.md  ", "spaced.md"],
  ])("accepts local path %j as %j", (href, expected) => {
    expect(localFilePath(href)).toBe(expected);
  });

  it.each([
    ["file:///Users/x/notes.md", "/Users/x/notes.md"],
    ["file://localhost/Users/x.md", "/Users/x.md"],
    // WHATWG file URLs expose a Windows drive as `/C:/…`; it must come back as
    // the drive-absolute spelling the host expects.
    ["file:///C:/work/a.ts", "C:/work/a.ts"],
    ["file://server/share/path/a.md", "\\\\server\\share\\path\\a.md"],
    // WHATWG lowercases the host component.
    ["file://SERVER/share", "\\\\server\\share"],
    // A host with an empty share has nothing to open.
    ["file://server/", null],
  ])("decodes file URL %j to %j", (href, expected) => {
    expect(localFilePath(href)).toBe(expected);
  });

  it("returns null for a malformed percent-encoding", () => {
    expect(localFilePath("file:///%E0%A4%A")).toBeNull();
  });

  it("treats a host-less file URL as the filesystem root", () => {
    // `new URL("file://")` is valid and yields pathname "/", so the helper
    // returns the root path rather than null. Documented, not a bug fix here.
    expect(localFilePath("file://")).toBe("/");
  });
});

describe("classifyMarkdownTarget", () => {
  it.each<[string, ReturnType<typeof classifyMarkdownTarget>]>([
    ["", { kind: "blocked" }],
    ["   ", { kind: "blocked" }],
    // A bare `#` is not an anchor.
    ["#", { kind: "blocked" }],
    ["#section-2", { anchor: "#section-2", kind: "document-anchor" }],
    ["docs/readme.md", { kind: "local-file", path: "docs/readme.md" }],
    ["/abs/a.md", { kind: "local-file", path: "/abs/a.md" }],
    ["https://example.com/a", { kind: "external-url", protocol: "https:", url: "https://example.com/a" }],
    ["http://example.com", { kind: "external-url", protocol: "http:", url: "http://example.com" }],
    ["mailto:me@example.com", { kind: "external-url", protocol: "mailto:", url: "mailto:me@example.com" }],
    // Not a supported scheme → inert.
    ["ftp://example.com/a", { kind: "blocked" }],
    ["futureos://file/1", { kind: "blocked" }],
    // Unparsable and scheme-less destinations remain inert.
    ["not a url", { kind: "blocked" }],
    ["//cdn.example.com/x", { kind: "blocked" }],
  ])("classifies %j", (value, expected) => {
    expect(classifyMarkdownTarget(value)).toEqual(expected);
  });

  it("trims before classifying", () => {
    expect(classifyMarkdownTarget("  #top  ")).toEqual({ anchor: "#top", kind: "document-anchor" });
  });
});

describe("remoteMarkdownImageUrl", () => {
  it("allows only http(s) images", () => {
    expect(remoteMarkdownImageUrl("https://example.com/a.png")).toBe("https://example.com/a.png");
    expect(remoteMarkdownImageUrl("http://example.com/a.png")).toBe("http://example.com/a.png");
    expect(remoteMarkdownImageUrl("mailto:me@example.com")).toBeNull();
    expect(remoteMarkdownImageUrl("docs/a.png")).toBeNull();
    expect(remoteMarkdownImageUrl("")).toBeNull();
  });
});

describe("basename", () => {
  it.each([
    ["a/b/c.md", "c.md"],
    ["C:\\dir\\file.txt", "file.txt"],
    ["mixed/dir\\file", "file"],
    ["a/", "a"],
    ["tail", "tail"],
    // No segment at all falls back to the raw input.
    ["/", "/"],
    ["", ""],
  ])("basename(%j) === %j", (path, expected) => {
    expect(basename(path)).toBe(expected);
  });
});
