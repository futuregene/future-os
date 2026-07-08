/**
 * Renders a unified git diff string as colored, line-numbered rows (old/new
 * line numbers, add/delete/context/meta tinting).
 */
export function DiffView({ diff }: { diff: string }) {
  const rows = diffRows(diff);

  return (
    <div className="max-h-[70vh] overflow-auto bg-surface font-mono text-xs leading-5">
      {rows.map(row => (
        <DiffLine
          key={row.key}
          kind={row.kind}
          line={row.line}
          newLineNumber={row.newLineNumber}
          oldLineNumber={row.oldLineNumber}
        />
      ))}
    </div>
  );
}

function DiffLine({
  kind,
  line,
  newLineNumber,
  oldLineNumber,
}: {
  kind: string;
  line: string;
  newLineNumber?: number;
  oldLineNumber?: number;
}) {
  const content = line.length === 0 ? " " : line;

  return (
    <div className={diffLineClass(kind)}>
      <span className="w-16 shrink-0 select-none border-r border-line-soft px-1.5 text-right text-ink-muted">
        {oldLineNumber ?? ""}
        <span className="inline-block w-2" />
        {newLineNumber ?? ""}
      </span>
      <code className="min-w-0 flex-1 whitespace-pre-wrap wrap-break-word px-3">{content}</code>
    </div>
  );
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

function diffLineClass(kind: string) {
  const base = "flex min-w-0 border-l-2";
  switch (kind) {
    case "add":
      return `${base} border-diff-add-line bg-diff-add text-success`;
    case "delete":
      return `${base} border-diff-remove-line bg-diff-remove text-danger`;
    case "meta":
      return `${base} border-transparent bg-surface-subtle text-ink-muted`;
    default:
      return `${base} border-transparent text-ink-soft`;
  }
}
