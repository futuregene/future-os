import {
  externalMimeCandidates,
  MAX_JSON_RICH_PREVIEW_BYTES,
  mobileFileType,
  mobilePreviewRoute,
} from "../fileTypes";

describe("mobile file type policy", () => {
  test.each([
    ["report.DOCX",
      "external",
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ],
    // Text data formats move bytes the reader can render itself, so they stay
    // in-app instead of being handed to another app.
    ["data.csv", "text", "text/csv"],
    ["events.jsonl", "text", "application/jsonl"],
    ["settings.yaml", "text", "application/yaml"],
    ["rows.ndjson", "text", "application/x-ndjson"],
    ["feed.xml", "text", "application/xml"],
    ["safe.html", "external", "text/html"],
    ["drawing.svg", "external", "image/svg+xml"],
    ["results.json", "json", "application/json"],
    ["notebook.ipynb", "json", "application/json"],
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
    ["values.yaml", "text", "application/yaml"],
    ["api.proto", "text", "text/plain"],
    ["changes.patch", "text", "text/plain"],
    ["review.diff", "text", "text/plain"],
    ["schema.graphql", "text", "text/plain"],
    ["Main.elm", "text", "text/plain"],
    ["legacy.f", "text", "text/plain"],
    ["AppDelegate.mm", "text", "text/plain"],
    ["deploy.hcl", "text", "text/plain"],
    ["prod.tfvars", "text", "text/plain"],
  ])("classifies %s", (name, route, mimeType) => {
    expect(mobileFileType(name)).toMatchObject({ route, mimeType });
  });

  test.each([
    // Build entry points, manifests and dotfiles have no suffix to match on.
    ["Makefile", "text", "text/plain"],
    ["GNUmakefile", "text", "text/plain"],
    ["Dockerfile", "text", "text/plain"],
    ["Dockerfile.dev", "text", "text/plain"],
    ["Jenkinsfile", "text", "text/plain"],
    ["CMakeLists.txt", "text", "text/plain"],
    ["build.mk", "text", "text/plain"],
    ["Cargo.lock", "text", "text/plain"],
    ["go.mod", "text", "text/plain"],
    [".gitignore", "text", "text/plain"],
    [".bashrc", "text", "text/plain"],
    [".env.local", "text", "text/plain"],
    ["LICENSE", "text", "text/plain"],
    ["README", "text", "text/plain"],
    ["Podfile", "text", "text/plain"],
    ["meson.build", "text", "text/plain"],
  ])("classifies the suffix-less %s", (name, route, mimeType) => {
    expect(mobileFileType(name)).toMatchObject({ route, mimeType });
  });

  test("only the base name decides the type", () => {
    expect(mobileFileType("/w/Makefile")).toMatchObject({ route: "text" });
    expect(mobileFileType("C:\\w\\servers\\Makefile")).toMatchObject({ route: "text" });
    // A directory that happens to end in a known suffix is not the file's type.
    expect(mobileFileType("/w/notes.md/scratch")).toBeNull();
    expect(mobileFileType("/w/main.rs/notes")).toBeNull();
  });

  test("rejects files outside the business allow-list", () => {
    expect(mobileFileType("dataset.h5")).toBeNull();
    expect(mobileFileType("program.exe")).toBeNull();
    expect(mobileFileType("archive.bin")).toBeNull();
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

  test("reads the notebook as JSON until the rich ceiling, then as text", () => {
    expect(mobilePreviewRoute("notes.ipynb", 4096)).toBe("json");
    expect(mobilePreviewRoute("notes.ipynb", MAX_JSON_RICH_PREVIEW_BYTES)).toBe("text");
  });

  test("uses rich JSON only below the 1 MiB boundary", () => {
    expect(mobilePreviewRoute("data.json", MAX_JSON_RICH_PREVIEW_BYTES - 1)).toBe("json");
    expect(mobilePreviewRoute("data.json", MAX_JSON_RICH_PREVIEW_BYTES)).toBe("text");
    expect(mobilePreviewRoute("data.json", MAX_JSON_RICH_PREVIEW_BYTES + 1)).toBe("text");
  });

  test("a file the phone cannot classify has no preview route to offer", () => {
    expect(mobilePreviewRoute("archive.zzz9", 4096)).toBeNull();
    expect(mobilePreviewRoute("", 0)).toBeNull();
    // A desktop path is decided by its last segment, on either separator.
    expect(mobilePreviewRoute("C:\\docs\\report.zzz9", 4096)).toBeNull();
    expect(mobilePreviewRoute("C:\\docs\\notes.txt", 4096)).toBe("text");
  });
});
