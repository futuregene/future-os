/**
 * Renders a unified git diff string as colored, single-gutter rows. Long code
 * lines remain intact and are viewed through horizontal scrolling.
 */
export function DiffView({ diff }: { diff: string }) {
  const rows = diffRows(diff);
  // Mirror Codex's render guardrails: on large diffs, retain the plain-text
  // diff but let the browser defer layout/paint for off-screen rows.
  const deferOffscreenRows = diff.length > 512 * 1024 || rows.length > 10_000 || rows.some(row => row.line.length > 4 * 1024);

  return (
    <div className="flex min-w-0 bg-surface font-mono text-xs leading-5">
      <div className="shrink-0">
        {rows.map(row => (
          <DiffGutter key={row.key} kind={row.kind} lineNumber={displayLineNumber(row.kind, row.oldLineNumber, row.newLineNumber)} />
        ))}
      </div>
      <div className="min-w-0 flex-1 overflow-x-auto overflow-y-hidden">
        <div className="min-w-full w-max">
          {rows.map(row => (
            <DiffContent
              deferOffscreen={deferOffscreenRows}
              key={row.key}
              kind={row.kind}
              line={row.line}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

function DiffGutter({
  kind,
  lineNumber,
}: {
  kind: string;
  lineNumber: number | "";
}) {
  return (
    <span className={diffGutterClass(kind)}>{lineNumber}</span>
  );
}

function DiffContent({
  kind,
  line,
  deferOffscreen,
}: {
  kind: string;
  line: string;
  deferOffscreen: boolean;
}) {
  const content = line.length === 0 ? " " : line;

  return (
    <code
      className={diffContentClass(kind)}
      style={deferOffscreen ? { containIntrinsicSize: "20px", contentVisibility: "auto" } : undefined}
    >
      {content}
    </code>
  );
}

function displayLineNumber(kind: string, oldLineNumber?: number, newLineNumber?: number) {
  if (kind === "delete")
    return oldLineNumber ?? "";
  return newLineNumber ?? "";
}

function diffRows(diff: string) {
  const seen = new Map<string, number>();
  let oldLine = 0;
  let newLine = 0;
  // `---`/`+++` are file-header meta only up to the first hunk; after that a
  // leading `--`/`++` belongs to a real deleted/added line (e.g. SQL comments).
  let hasHunk = false;
  return diff
    .split("\n")
    .filter(line => !line.startsWith("diff --git ") && !line.startsWith("index "))
    .map((line) => {
      const count = (seen.get(line) ?? 0) + 1;
      seen.set(line, count);
      const hunk = parseHunkHeader(line);
      if (hunk) {
        oldLine = hunk.oldStart;
        newLine = hunk.newStart;
      }

      let oldLineNumber: number | undefined;
      let newLineNumber: number | undefined;
      const kind = diffLineKind(line, hasHunk);
      if (hunk)
        hasHunk = true;
      if (kind === "add") {
        newLineNumber = newLine;
        newLine += 1;
      }
      else if (kind === "delete") {
        oldLineNumber = oldLine;
        oldLine += 1;
      }
      else if (kind === "context") {
        oldLineNumber = oldLine;
        newLineNumber = newLine;
        oldLine += 1;
        newLine += 1;
      }
      return {
        key: `${count}:${line}`,
        kind,
        line,
        newLineNumber,
        oldLineNumber,
      };
    });
}

function parseHunkHeader(line: string) {
  const match = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
  if (!match)
    return null;
  return {
    // Groups 1 and 2 are required `(\d+)` captures — present whenever `match` is.
    newStart: Number.parseInt(match[2]!, 10),
    oldStart: Number.parseInt(match[1]!, 10),
  };
}

function diffLineKind(line: string, hasHunk: boolean) {
  if (line.startsWith("@@") || line.startsWith("new file")) {
    return "meta";
  }
  // A `---`/`+++` line is a file header only in the pre-hunk header block or when
  // it carries a git `a/`, `b/`, or `/dev/null` path; otherwise it's diff content.
  if (line.startsWith("---") || line.startsWith("+++")) {
    if (!hasHunk || /^(?:---|\+\+\+) (?:a\/|b\/|\/dev\/null)/.test(line)) {
      return "meta";
    }
  }
  if (line.startsWith("+")) {
    return "add";
  }
  if (line.startsWith("-")) {
    return "delete";
  }
  return "context";
}

function diffGutterClass(kind: string) {
  const base = "block h-5 w-10 select-none border-r border-line-soft border-l-2 px-1.5 text-right text-ink-muted";
  switch (kind) {
    case "add":
      return `${base} border-l-diff-add-line bg-diff-add text-success`;
    case "delete":
      return `${base} border-l-diff-remove-line bg-diff-remove text-danger`;
    case "meta":
      return `${base} border-transparent bg-surface-subtle text-ink-muted`;
    default:
      return `${base} border-transparent text-ink-soft`;
  }
}

function diffContentClass(kind: string) {
  const base = "block h-5 min-w-full w-max whitespace-pre px-3";
  switch (kind) {
    case "add":
      return `${base} bg-diff-add text-success`;
    case "delete":
      return `${base} bg-diff-remove text-danger`;
    case "meta":
      return `${base} bg-surface-subtle text-ink-muted`;
    default:
      return `${base} text-ink-soft`;
  }
}
