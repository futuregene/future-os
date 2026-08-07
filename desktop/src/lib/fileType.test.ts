import { describe, expect, it } from "vitest";
import { fileKind } from "./fileType";

describe("fileKind", () => {
  it("returns folder for directories regardless of the name", () => {
    expect(fileKind("src", true)).toBe("folder");
    expect(fileKind("notes.md", true)).toBe("folder");
  });

  it("classifies images", () => {
    expect(fileKind("photo.PNG")).toBe("image");
    expect(fileKind("a/b/icon.svg")).toBe("image");
    expect(fileKind("scan.jpeg")).toBe("image");
  });

  it("classifies pdf / markdown / html before the generic fallbacks", () => {
    expect(fileKind("report.pdf")).toBe("pdf");
    expect(fileKind("README.md")).toBe("markdown");
    expect(fileKind("notes.markdown")).toBe("markdown");
    expect(fileKind("index.html")).toBe("html");
  });

  it("classifies archives and shell scripts", () => {
    expect(fileKind("bundle.tar.gz")).toBe("archive");
    expect(fileKind("build.zip")).toBe("archive");
    expect(fileKind("deploy.sh")).toBe("shell");
    expect(fileKind("run.ps1")).toBe("shell");
  });

  it("classifies code by extension", () => {
    expect(fileKind("main.rs")).toBe("code");
    expect(fileKind("app.tsx")).toBe("code");
    expect(fileKind("data.json")).toBe("code");
  });

  it("falls back to text for unknown or extension-less names", () => {
    expect(fileKind("LICENSE")).toBe("text");
    expect(fileKind(".bashrc")).toBe("text");
    expect(fileKind("notes.unknownext")).toBe("text");
    expect(fileKind("")).toBe("text");
  });

  it("ignores dots in parent directories", () => {
    expect(fileKind("v1.2.3/Makefile")).toBe("text");
    expect(fileKind("a.b.c/photo.png")).toBe("image");
  });
});
