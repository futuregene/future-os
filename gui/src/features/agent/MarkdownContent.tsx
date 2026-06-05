import type { ReactNode } from "react";

interface MarkdownContentProps {
  content: string;
}

type Block
  = | { text: string; type: "heading"; level: 1 | 2 | 3 }
    | { text: string; type: "paragraph" }
    | { code: string; language?: string; type: "code" }
    | { items: string[]; ordered: boolean; type: "list" };

export function MarkdownContent({ content }: MarkdownContentProps) {
  return <div className="space-y-3">{parseBlocks(content).map(renderBlock)}</div>;
}

function renderBlock(block: Block, index: number) {
  switch (block.type) {
    case "heading": {
      const className = block.level === 1
        ? "text-lg font-semibold leading-7 text-ink"
        : block.level === 2
          ? "text-base font-semibold leading-7 text-ink"
          : "text-sm font-semibold leading-6 text-ink";
      const children = renderInline(block.text);
      if (block.level === 1)
        return <h1 className={className} key={index}>{children}</h1>;
      if (block.level === 2)
        return <h2 className={className} key={index}>{children}</h2>;
      return <h3 className={className} key={index}>{children}</h3>;
    }
    case "list": {
      const Tag = block.ordered ? "ol" : "ul";
      return (
        <Tag
          className={block.ordered
            ? "list-decimal space-y-1 pl-5"
            : "list-disc space-y-1 pl-5"}
          key={index}
        >
          {block.items.map(item => (
            <li className="pl-1" key={item}>{renderInline(item)}</li>
          ))}
        </Tag>
      );
    }
    case "code":
      return (
        <pre className="overflow-auto rounded-lg bg-slate-100 p-3 text-xs leading-5 text-ink" key={index}>
          {block.language ? <div className="mb-2 text-[11px] text-ink-muted">{block.language}</div> : null}
          <code>{block.code}</code>
        </pre>
      );
    default:
      return (
        <p className="whitespace-pre-wrap" key={index}>
          {renderInline(block.text)}
        </p>
      );
  }
}

function parseBlocks(content: string): Block[] {
  const lines = content.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let paragraph: string[] = [];
  let list: { items: string[]; ordered: boolean } | null = null;
  let code: { language?: string; lines: string[] } | null = null;

  function flushParagraph() {
    if (paragraph.length === 0)
      return;
    blocks.push({ text: paragraph.join(" ").trim(), type: "paragraph" });
    paragraph = [];
  }

  function flushList() {
    if (!list)
      return;
    blocks.push({ items: list.items, ordered: list.ordered, type: "list" });
    list = null;
  }

  for (const line of lines) {
    const fence = line.match(/^```(\S*)\s*$/);
    if (fence) {
      if (code) {
        blocks.push({ code: code.lines.join("\n"), language: code.language, type: "code" });
        code = null;
      }
      else {
        flushParagraph();
        flushList();
        code = { language: fence[1] || undefined, lines: [] };
      }
      continue;
    }

    if (code) {
      code.lines.push(line);
      continue;
    }

    if (!line.trim()) {
      flushParagraph();
      flushList();
      continue;
    }

    const heading = parseHeading(line);
    if (heading) {
      flushParagraph();
      flushList();
      blocks.push({
        level: heading.level,
        text: heading.text,
        type: "heading",
      });
      continue;
    }

    const listItem = parseListItem(line);
    if (listItem) {
      flushParagraph();
      if (!list || list.ordered !== listItem.ordered) {
        flushList();
        list = { items: [], ordered: listItem.ordered };
      }
      list.items.push(listItem.text);
      continue;
    }

    flushList();
    paragraph.push(line.trim());
  }

  flushParagraph();
  flushList();
  if (code) {
    blocks.push({ code: code.lines.join("\n"), language: code.language, type: "code" });
  }

  return blocks.length > 0 ? blocks : [{ text: content, type: "paragraph" }];
}

function renderInline(text: string) {
  const parts = text.split(/(`[^`]+`|\*\*[^*]+\*\*)/g).filter(Boolean);
  return parts.map<ReactNode>((part) => {
    if (part.startsWith("**") && part.endsWith("**")) {
      return <strong className="font-semibold text-ink" key={`strong:${part}`}>{part.slice(2, -2)}</strong>;
    }
    if (part.startsWith("`") && part.endsWith("`")) {
      return (
        <code className="rounded bg-slate-100 px-1 py-0.5 text-[0.92em] text-ink" key={`code:${part}`}>
          {part.slice(1, -1)}
        </code>
      );
    }
    return part;
  });
}

function parseHeading(line: string) {
  const trimmed = line.trimStart();
  const markerLength = trimmed.match(/^#{1,3}/)?.[0].length ?? 0;
  if (markerLength === 0 || trimmed[markerLength] !== " ")
    return null;

  return {
    level: markerLength as 1 | 2 | 3,
    text: trimmed.slice(markerLength + 1).trim(),
  };
}

function parseListItem(line: string) {
  const trimmed = line.trimStart();
  const first = trimmed[0];
  if ((first === "-" || first === "*") && trimmed[1] === " ") {
    return { ordered: false, text: trimmed.slice(2).trim() };
  }

  const dotIndex = trimmed.indexOf(".");
  if (dotIndex <= 0 || trimmed[dotIndex + 1] !== " ")
    return null;

  const prefix = trimmed.slice(0, dotIndex);
  if (!/^\d+$/.test(prefix))
    return null;

  return { ordered: true, text: trimmed.slice(dotIndex + 2).trim() };
}
