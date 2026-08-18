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
  ])("classifies %s", (name, route, mimeType) => {
    expect(mobileFileType(name)).toMatchObject({ route, mimeType });
  });

  test("rejects files outside the business allow-list", () => {
    expect(mobileFileType("dataset.h5")).toBeNull();
    expect(mobileFileType("program.exe")).toBeNull();
    expect(mobileFileType("README")).toBeNull();
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
