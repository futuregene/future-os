import {
  externalMimeCandidates,
  MAX_JSON_RICH_PREVIEW_BYTES,
  mobileFileType,
  mobilePreviewRoute,
} from "../fileTypes";

describe("mobile file type policy", () => {
  test.each([
    [
      "report.DOCX",
      "external",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ],
    ["data.csv", "external", "text/csv"],
    ["events.jsonl", "external", "application/jsonl"],
    ["settings.yaml", "external", "application/yaml"],
    ["safe.html", "external", "text/html"],
    ["drawing.svg", "external", "image/svg+xml"],
    ["results.json", "json", "application/json"],
    ["archive.tar.gz", "external", "application/gzip"],
    ["archive.7z", "external", "application/x-7z-compressed"],
    ["paper.pages", "external", "application/vnd.apple.pages"],
    ["main.rs", "text", "text/plain"],
    ["server.go", "text", "text/plain"],
    ["train.py", "text", "text/x-python"],
    ["analysis.R", "text", "text/x-r-source"],
    ["query.sql", "text", "application/sql"],
    ["deploy.sh", "text", "application/x-sh"],
    ["Component.TSX", "text", "text/plain"],
    ["config.toml", "text", "text/plain"],
    ["values.yaml", "external", "application/yaml"],
  ])("classifies %s", (name, route, mimeType) => {
    expect(mobileFileType(name)).toMatchObject({ route, mimeType });
  });

  test("rejects files outside the business allow-list", () => {
    expect(mobileFileType("dataset.h5")).toBeNull();
    expect(mobileFileType("program.exe")).toBeNull();
    expect(mobileFileType("README")).toBeNull();
  });

  test("routes code and config files to the in-app text reader", () => {
    for (const name of ["lib.rs", "main.go", "app.java", "index.ts", "script.ps1", "cfg.env"]) {
      expect(mobilePreviewRoute(name, 4096)).toBe("text");
    }
    // JSON above the rich-preview ceiling still falls back to plain text.
    expect(mobilePreviewRoute("main.rs", MAX_JSON_RICH_PREVIEW_BYTES)).toBe("text");
  });

  test("offers text/plain only for approved text-like external formats", () => {
    expect(externalMimeCandidates("table.csv")).toEqual(["text/csv", "text/plain"]);
    expect(externalMimeCandidates("config.yml")).toEqual(["application/yaml", "text/plain"]);
    expect(externalMimeCandidates("page.html")).toEqual(["text/html"]);
    expect(externalMimeCandidates("vector.svg")).toEqual(["image/svg+xml"]);
    expect(externalMimeCandidates("archive.zip")).toEqual(["application/zip"]);
  });

  test("uses rich JSON only below the 1 MiB boundary", () => {
    expect(mobilePreviewRoute("data.json", MAX_JSON_RICH_PREVIEW_BYTES - 1)).toBe("json");
    expect(mobilePreviewRoute("data.json", MAX_JSON_RICH_PREVIEW_BYTES)).toBe("text");
    expect(mobilePreviewRoute("data.json", MAX_JSON_RICH_PREVIEW_BYTES + 1)).toBe("text");
  });
});
