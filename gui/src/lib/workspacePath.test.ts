import { describe, expect, it } from "vitest";
import { pathBasename, pathExtension, relativizeWorkspacePath } from "./workspacePath";

describe("pathBasename", () => {
  it("returns the last segment for POSIX and Windows separators", () => {
    expect(pathBasename("/home/user/report.md")).toBe("report.md");
    expect(pathBasename("C:\\Users\\me\\report.md")).toBe("report.md");
    expect(pathBasename("mixed/dir\\file.txt")).toBe("file.txt");
  });

  it("ignores trailing separators", () => {
    expect(pathBasename("/home/user/")).toBe("user");
    expect(pathBasename("C:\\Users\\me\\\\")).toBe("me");
  });

  it("returns a bare name unchanged", () => {
    expect(pathBasename("file.txt")).toBe("file.txt");
  });

  it("returns empty for a path with no segment", () => {
    expect(pathBasename("")).toBe("");
    expect(pathBasename("///")).toBe("");
  });
});

describe("pathExtension", () => {
  it("lowercases the extension of the last segment", () => {
    expect(pathExtension("/a/b/photo.PNG")).toBe("png");
    expect(pathExtension("C:\\pics\\photo.JpG")).toBe("jpg");
  });

  it("uses only the final dot", () => {
    expect(pathExtension("archive.tar.gz")).toBe("gz");
  });

  it("does not leak a dot from a parent directory", () => {
    expect(pathExtension("/home/v1.2/README")).toBe("");
  });

  it("treats a leading-dot name as having no extension", () => {
    expect(pathExtension("/home/user/.bashrc")).toBe("");
    expect(pathExtension(".gitignore")).toBe("");
  });

  it("returns empty when there is no extension", () => {
    expect(pathExtension("/usr/bin/node")).toBe("");
    expect(pathExtension("")).toBe("");
  });
});

describe("relativizeWorkspacePath", () => {
  const root = "/home/user/project";

  it("returns the path unchanged without a workspace root", () => {
    expect(relativizeWorkspacePath("/home/user/project/src/a.ts", null)).toBe("/home/user/project/src/a.ts");
    expect(relativizeWorkspacePath("/x/y.ts")).toBe("/x/y.ts");
  });

  it("relativizes a path inside the workspace", () => {
    expect(relativizeWorkspacePath("/home/user/project/src/a.ts", root)).toBe("src/a.ts");
  });

  it("tolerates a trailing separator on the root", () => {
    expect(relativizeWorkspacePath("/home/user/project/src/a.ts", "/home/user/project/")).toBe("src/a.ts");
  });

  it("keeps the workspace root itself absolute", () => {
    expect(relativizeWorkspacePath(root, root)).toBe(root);
  });

  it("keeps a path outside the workspace absolute", () => {
    expect(relativizeWorkspacePath("/home/user/other/a.ts", root)).toBe("/home/user/other/a.ts");
  });

  it("does not relativize a sibling that shares a name prefix", () => {
    expect(relativizeWorkspacePath("/home/user/project-2/a.ts", root)).toBe("/home/user/project-2/a.ts");
  });
});
