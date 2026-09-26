// @vitest-environment jsdom
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { DiffView } from "./DiffView";

/** Lines of the rendered output, one per `<code>` row, in document order. */
function contentRows(html: string): string[] {
  return [...html.matchAll(/<code[^>]*>([\s\S]*?)<\/code>/g)].map(match => decodeEntities(match[1]!));
}

function gutterNumbers(html: string): string[] {
  return [...html.matchAll(/<span class="[^"]*">([^<]*)<\/span>/g)].map(match => match[1]!);
}

function decodeEntities(text: string) {
  return text
    .split("&lt;")
    .join("<")
    .split("&gt;")
    .join(">")
    .split("&quot;")
    .join("\"")
    .split("&#x27;")
    .join("'")
    .split("&amp;")
    .join("&");
}

const SAMPLE = [
  "diff --git a/src/app.ts b/src/app.ts",
  "index 1111111..2222222 100644",
  "--- a/src/app.ts",
  "+++ b/src/app.ts",
  "@@ -1,3 +1,4 @@",
  " const kept = 1;",
  "-const removed = 2;",
  "+const added = 3;",
  "+const second = 4;",
].join("\n");

describe("diffView structure", () => {
  it("renders one gutter cell and one content row per surviving diff line", () => {
    const html = renderToStaticMarkup(createElement(DiffView, { diff: SAMPLE }));
    // `diff --git` and `index` are dropped; the remaining six lines each render.
    expect(contentRows(html)).toEqual([
      "--- a/src/app.ts",
      "+++ b/src/app.ts",
      "@@ -1,3 +1,4 @@",
      " const kept = 1;",
      "-const removed = 2;",
      "+const added = 3;",
      "+const second = 4;",
    ]);
    expect(gutterNumbers(html)).toHaveLength(7);
  });

  it("numbers added rows by the new file and deleted rows by the old file", () => {
    const html = renderToStaticMarkup(createElement(DiffView, { diff: SAMPLE }));
    // Gutter order: header, header, hunk, context(1/1), delete(2), add(2), add(3).
    expect(gutterNumbers(html)).toEqual(["", "", "", "1", "2", "2", "3"]);
  });

  it("classifies every row kind with its own colours", () => {
    const html = renderToStaticMarkup(createElement(DiffView, { diff: SAMPLE }));
    expect(html).toContain("bg-diff-add text-success");
    expect(html).toContain("bg-diff-remove text-danger");
    expect(html).toContain("bg-surface-subtle text-ink-muted");
    // Context rows keep the neutral body colour.
    expect(html).toContain("text-ink-soft");
  });

  it("treats a `new file` marker as meta", () => {
    const html = renderToStaticMarkup(createElement(DiffView, {
      diff: ["new file mode 100644", "@@ -0,0 +1,1 @@", "+hello"].join("\n"),
    }));
    expect(contentRows(html)).toEqual(["new file mode 100644", "@@ -0,0 +1,1 @@", "+hello"]);
    expect(gutterNumbers(html)).toEqual(["", "", "1"]);
  });
});

describe("diffView header-vs-content disambiguation", () => {
  it("keeps `---`/`+++` as meta while the path is a git a/ b/ path", () => {
    const html = renderToStaticMarkup(createElement(DiffView, {
      diff: ["@@ -1,1 +1,1 @@", "--- a/x", "+++ b/x", "--- not a path"].join("\n"),
    }));
    // The first two stay meta (empty gutters); the third is a real deletion.
    expect(gutterNumbers(html)).toEqual(["", "", "", "1"]);
  });

  it("treats `---`/`+++` after a hunk as real content, not a header", () => {
    // A SQL comment (`-- ...`) and a `++` line are content, not file headers.
    const html = renderToStaticMarkup(createElement(DiffView, {
      diff: ["@@ -5,2 +5,2 @@", "-- sql comment", "++doubled"].join("\n"),
    }));
    const rows = contentRows(html);
    expect(rows).toEqual(["@@ -5,2 +5,2 @@", "-- sql comment", "++doubled"]);
    // Both are counted as real lines (delete at old 5, add at new 5).
    expect(gutterNumbers(html)).toEqual(["", "5", "5"]);
  });

  it("treats a `/dev/null` header as meta", () => {
    const html = renderToStaticMarkup(createElement(DiffView, {
      diff: ["@@ -1,1 +1,1 @@", "--- /dev/null", "+++ b/add.ts"].join("\n"),
    }));
    expect(gutterNumbers(html)).toEqual(["", "", ""]);
  });

  it("keys repeated identical lines so React keeps them distinct", () => {
    const html = renderToStaticMarkup(createElement(DiffView, {
      diff: ["@@ -1,4 +1,4 @@", " same", " same", " same"].join("\n"),
    }));
    expect(gutterNumbers(html)).toEqual(["", "1", "2", "3"]);
  });
});

describe("diffView boundaries", () => {
  it("renders a single placeholder row for an empty diff", () => {
    // `"".split("\n")` yields one empty line; it is drawn with the blank-body
    // substitute so the block keeps its line height instead of collapsing.
    expect(contentRows(renderToStaticMarkup(createElement(DiffView, { diff: "" })))).toEqual([" "]);
  });

  it("substitutes a single space for an empty content line", () => {
    const html = renderToStaticMarkup(createElement(DiffView, {
      diff: ["@@ -1,2 +1,2 @@", " kept", ""].join("\n"),
    }));
    const rows = contentRows(html);
    expect(rows[rows.length - 1]).toBe(" ");
    // The blank context line still advances both sides' counters.
    expect(gutterNumbers(html)).toEqual(["", "1", "2"]);
  });

  it("keeps very long lines intact and defers their offscreen layout", () => {
    const huge = `+${"x".repeat(4200)}`;
    const html = renderToStaticMarkup(createElement(DiffView, { diff: huge }));
    expect(contentRows(html)).toEqual([huge]);
    expect(html).toContain("content-visibility:auto");
    expect(html).toContain("contain-intrinsic-size:20px");
  });

  it("does not defer layout for an ordinary diff", () => {
    const html = renderToStaticMarkup(createElement(DiffView, { diff: "+short" }));
    expect(html).not.toContain("content-visibility:auto");
  });

  it("passes CJK and emoji content through unchanged", () => {
    const html = renderToStaticMarkup(createElement(DiffView, {
      diff: ["@@ -1,2 +1,2 @@", "-中文标题", "+emoji 🚀 行"].join("\n"),
    }));
    expect(contentRows(html)).toEqual(["@@ -1,2 +1,2 @@", "-中文标题", "+emoji 🚀 行"]);
  });

  it("defers layout when the diff exceeds the size budget", () => {
    const html = renderToStaticMarkup(createElement(DiffView, { diff: `+${"y".repeat(512 * 1024 + 1)}` }));
    expect(html).toContain("content-visibility:auto");
  });

  it("keeps numbering monotonic across two hunks", () => {
    const html = renderToStaticMarkup(createElement(DiffView, {
      diff: [
        "@@ -1,1 +1,1 @@",
        " a",
        "@@ -50,1 +60,1 @@",
        "+b",
      ].join("\n"),
    }));
    expect(gutterNumbers(html)).toEqual(["", "1", "", "60"]);
  });
});
