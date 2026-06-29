import type { ReactNode } from "react";
import type { ResolvedMarkdownReference } from "../../integrations/storage/markdownReferences";
import type {
  StoredApprovalRequest,
  StoredArtifact,
  StoredResearchResource,
  StoredReviewChangeset,
  StoredRun,
  StoredToolCall,
} from "../../integrations/storage/types";
import type { FutureReference, InlineNode, MarkdownNode } from "./futureMarkdownTypes";
import { Check, Clipboard } from "lucide-react";
import { useMemo, useState } from "react";
import { copyText } from "../../lib/clipboard";
import { useFutureReference, useFutureReferences } from "./futureReferenceStore";
import { parseFutureMarkdown } from "./parseFutureMarkdown";
import { ArtifactEmbed } from "./renderers/ArtifactEmbed";
import { MissingReference } from "./renderers/MissingReference";
import { ApprovalEmbed, ResearchEmbed, ReviewEmbed, ToolEmbed } from "./renderers/ObjectEmbed";
import { ReferenceChip } from "./renderers/ReferenceChip";
import { RunEmbed } from "./renderers/RunEmbed";
import { useCodeHighlighter } from "./useCodeHighlighter";

interface MarkdownContentProps {
  content: string;
  workspaceId?: string | null;
}

export function MarkdownContent({ content, workspaceId }: MarkdownContentProps) {
  const document = useMemo(() => parseFutureMarkdown(content), [content]);
  useFutureReferences(workspaceId, document.references);

  return <div className="space-y-3">{document.nodes.map((node, index) => renderBlock(node, workspaceId, `b${index}`))}</div>;
}

function renderBlock(node: MarkdownNode, workspaceId: string | null | undefined, key: string) {
  switch (node.type) {
    case "heading": {
      const className = node.level === 1
        ? "text-lg font-semibold leading-7 text-ink"
        : node.level === 2
          ? "text-base font-semibold leading-7 text-ink"
          : "text-sm font-semibold leading-6 text-ink";
      const children = renderInline(node.children, workspaceId, key);
      if (node.level === 1)
        return <h1 className={className} key={key}>{children}</h1>;
      if (node.level === 2)
        return <h2 className={className} key={key}>{children}</h2>;
      return <h3 className={className} key={key}>{children}</h3>;
    }
    case "list": {
      const Tag = node.ordered ? "ol" : "ul";
      return (
        <Tag
          className={node.ordered
            ? "list-decimal space-y-1 pl-5"
            : "list-disc space-y-1 pl-5"}
          key={key}
        >
          {withStableKeys(node.items, key).map(({ item, key: itemKey }) => (
            <li className="pl-1" key={itemKey}>
              <ListItemContent item={item} itemKey={itemKey} workspaceId={workspaceId} />
            </li>
          ))}
        </Tag>
      );
    }
    case "code":
      return <CodeBlock code={node.code} key={key} language={node.language} />;
    case "futureEmbed":
      return <FutureEmbedView key={key} reference={node.reference} workspaceId={workspaceId} />;
    case "blockquote":
      return (
        <blockquote className="border-l-2 border-line-strong pl-3 text-ink-soft" key={key}>
          <div className="space-y-2">
            {node.children.map((child, ordinal) => renderBlock(child, workspaceId, `${key}:q${ordinal}`))}
          </div>
        </blockquote>
      );
    case "table":
      return (
        <div className="overflow-x-auto" key={key}>
          <table className="min-w-full border-separate border-spacing-0 overflow-hidden rounded-md border border-line-soft text-sm">
            <thead className="bg-surface-subtle text-ink">
              <tr>
                {withStableKeys(node.headers, `${key}:h`).map(({ item: header, key: headerKey, ordinal: column }) => (
                  <th
                    className="border-b border-line-soft px-3 py-2 text-left font-semibold"
                    key={headerKey}
                    style={{ textAlign: tableTextAlign(node.alignments[column]) }}
                  >
                    {renderInline(header, workspaceId, headerKey)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {withStableKeys(node.rows, `${key}:r`).map(({ item: row, key: rowKey }) => (
                <tr className="odd:bg-surface even:bg-surface-subtle/60" key={rowKey}>
                  {withStableKeys(row, `${rowKey}:c`).map(({ item: cell, key: cellKey, ordinal: column }) => (
                    <td
                      className="border-b border-line-soft px-3 py-2 align-top text-ink-soft last:border-b-0"
                      key={cellKey}
                      style={{ textAlign: tableTextAlign(node.alignments[column]) }}
                    >
                      {renderInline(cell, workspaceId, cellKey)}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "thematicBreak":
      return <hr className="border-line-soft" key={key} />;
    default:
      return (
        <p className="whitespace-pre-wrap" key={key}>
          {renderInline(node.children, workspaceId, key)}
        </p>
      );
  }
}

function CodeBlock({
  code,
  language,
}: {
  code: string;
  language?: string;
}) {
  const [copied, setCopied] = useState(false);
  const { highlight, isLoaded } = useCodeHighlighter();
  const highlighted = useMemo(() => highlight(code, language), [highlight, code, language]);

  async function handleCopy() {
    await copyText(code);
    setCopied(true);
    window.setTimeout(setCopied, 1400, false);
  }

  // Fallback to plain text if highlighter not loaded or language not supported
  if (!isLoaded || !highlighted) {
    return (
      <div className="relative">
        <button
          aria-label="Copy code"
          className="absolute right-1.5 top-1.5 inline-flex size-7 items-center justify-center rounded-md bg-surface/90 text-ink-muted shadow-sm ring-1 ring-line-soft transition-colors hover:text-ink"
          onClick={() => void handleCopy()}
          title="Copy code"
          type="button"
        >
          {copied ? <Check className="size-3.5" /> : <Clipboard className="size-3.5" />}
        </button>
        <pre className="overflow-auto rounded-lg bg-surface-subtle p-3 pr-11 text-xs leading-5 text-ink">
          {language ? <div className="mb-2 text-[11px] text-ink-muted">{language}</div> : null}
          <code>{code}</code>
        </pre>
      </div>
    );
  }

  return (
    <div className="relative">
      <button
        aria-label="Copy code"
        className="absolute right-1.5 top-1.5 z-10 inline-flex size-7 items-center justify-center rounded-md bg-surface/90 text-ink-muted shadow-sm ring-1 ring-line-soft transition-colors hover:text-ink"
        onClick={() => void handleCopy()}
        title="Copy code"
        type="button"
      >
        {copied ? <Check className="size-3.5" /> : <Clipboard className="size-3.5" />}
      </button>
      <pre
        className="overflow-auto rounded-lg p-3 pr-11 text-xs leading-5"
        style={{ backgroundColor: highlighted.bgColor, color: highlighted.fgColor }}
      >
        {language ? <div className="mb-2 text-[11px] opacity-60">{language}</div> : null}
        <code>
          {highlighted.lines.map((line, lineIndex) => (
            // eslint-disable-next-line react/no-array-index-key -- static positional render of highlighted code; lines never reorder
            <div key={lineIndex} className="flex">
              <span className="mr-4 inline-block w-8 select-none text-right opacity-40">
                {lineIndex + 1}
              </span>
              <span className="flex-1">
                {line.tokens.map((token, tokenIndex) => (
                  <span
                    key={tokenIndex} // eslint-disable-line react/no-array-index-key -- static positional render of highlighted tokens; index key is fine
                    style={{
                      color: token.color,
                      fontStyle: token.fontStyle ? (token.fontStyle & 1 ? "italic" : "normal") : undefined,
                    }}
                  >
                    {token.content}
                  </span>
                ))}
              </span>
            </div>
          ))}
        </code>
      </pre>
    </div>
  );
}

function renderInline(nodes: InlineNode[], workspaceId: string | null | undefined, parentKey: string) {
  return nodes.map<ReactNode>((node, index) => {
    const key = `${parentKey}:in${index}`;
    switch (node.type) {
      case "strong":
        return <strong className="font-semibold text-ink" key={key}>{renderInline(node.children, workspaceId, key)}</strong>;
      case "italic":
        return <em className="italic" key={key}>{renderInline(node.children, workspaceId, key)}</em>;
      case "delete":
        return <del className="text-ink-muted" key={key}>{renderInline(node.children, workspaceId, key)}</del>;
      case "code":
        return (
          <code className="rounded bg-surface-subtle px-1 py-0.5 text-[0.92em] text-ink" key={key}>
            {node.code}
          </code>
        );
      case "break":
        return <br key={key} />;
      case "link":
        return <SafeLink href={node.href} key={key}>{renderInline(node.children, workspaceId, key)}</SafeLink>;
      case "image":
        return <SafeImage alt={node.alt} key={key} src={node.src} title={node.title} />;
      case "futureReference":
        return <FutureReferenceChip key={key} reference={node.reference} workspaceId={workspaceId} />;
      default:
        return node.text;
    }
  });
}

function ListItemContent({
  item,
  itemKey,
  workspaceId,
}: {
  item: Extract<MarkdownNode, { type: "list" }>["items"][number];
  itemKey: string;
  workspaceId: string | null | undefined;
}) {
  const hasInlineContent = item.children.length > 0 || item.checked !== undefined;
  const content = item.checked === undefined
    ? renderInline(item.children, workspaceId, itemKey)
    : (
        <span className="inline-flex items-start gap-2">
          <input
            checked={item.checked}
            className="mt-1 size-3.5 accent-accent"
            readOnly
            type="checkbox"
          />
          <span>{renderInline(item.children, workspaceId, itemKey)}</span>
        </span>
      );

  if (!item.blocks?.length)
    return content;

  return (
    <div className="space-y-2">
      {hasInlineContent ? <div>{content}</div> : null}
      <div className="space-y-2">
        {item.blocks.map((block, ordinal) => renderBlock(block, workspaceId, `${itemKey}:b${ordinal}`))}
      </div>
    </div>
  );
}

function tableTextAlign(alignment: "center" | "left" | "right" | null | undefined) {
  return alignment ?? "left";
}

function withStableKeys<T>(items: T[], seed: string) {
  return items.map((item, ordinal) => ({
    item,
    key: `${seed}:${ordinal}`,
    ordinal,
  }));
}

function FutureReferenceChip({
  reference,
  workspaceId,
}: {
  reference: FutureReference;
  workspaceId: string | null | undefined;
}) {
  const resolved = useFutureReference(workspaceId, reference);
  return <ReferenceChip reference={reference} resolved={resolved} />;
}

function FutureEmbedView({
  reference,
  workspaceId,
}: {
  reference: FutureReference;
  workspaceId: string | null | undefined;
}) {
  const resolved = useFutureReference(workspaceId, reference);
  return <FutureEmbed reference={reference} resolved={resolved} />;
}

function FutureEmbed({
  reference,
  resolved,
}: {
  reference: FutureReference;
  resolved?: ResolvedMarkdownReference;
}) {
  if (!resolved || resolved.status !== "resolved") {
    return <MissingReference error={resolved?.error} reference={reference} />;
  }

  if (reference.targetType === "artifact" && resolved.targetType === "artifact") {
    if (isStoredArtifact(resolved.data)) {
      return <ArtifactEmbed artifact={resolved.data} reference={reference} />;
    }
    return <MissingReference error="artifact payload is invalid" reference={reference} />;
  }

  if (reference.targetType === "run" && resolved.targetType === "run") {
    if (isStoredRun(resolved.data)) {
      return <RunEmbed reference={reference} run={resolved.data} />;
    }
    return <MissingReference error="run payload is invalid" reference={reference} />;
  }

  if (reference.targetType === "approval" && resolved.targetType === "approval") {
    if (isStoredApproval(resolved.data)) {
      return <ApprovalEmbed approval={resolved.data} reference={reference} />;
    }
    return <MissingReference error="approval payload is invalid" reference={reference} />;
  }

  if (reference.targetType === "review" && resolved.targetType === "review") {
    if (isStoredReview(resolved.data)) {
      return <ReviewEmbed reference={reference} review={resolved.data} />;
    }
    return <MissingReference error="review payload is invalid" reference={reference} />;
  }

  if (reference.targetType === "research" && resolved.targetType === "research") {
    if (isStoredResearch(resolved.data)) {
      return <ResearchEmbed reference={reference} resource={resolved.data} />;
    }
    return <MissingReference error="research payload is invalid" reference={reference} />;
  }

  if (reference.targetType === "tool" && resolved.targetType === "tool") {
    if (isStoredTool(resolved.data)) {
      return <ToolEmbed reference={reference} tool={resolved.data} />;
    }
    return <MissingReference error="tool payload is invalid" reference={reference} />;
  }

  return <MissingReference error="reference type mismatch" reference={reference} />;
}

function SafeLink({
  children,
  href,
}: {
  children: ReactNode;
  href: string;
}) {
  const safeHref = safeExternalUrl(href, ["http:", "https:", "mailto:"]);
  if (!safeHref) {
    return <span className="font-medium text-ink-soft" title={href}>{children}</span>;
  }

  return (
    <a
      className="font-medium text-accent underline-offset-2 hover:underline"
      href={safeHref}
      rel="noopener noreferrer"
      target="_blank"
      title={href}
    >
      {children}
    </a>
  );
}

function SafeImage({
  alt,
  src,
  title,
}: {
  alt: string;
  src: string;
  title?: string;
}) {
  const [failed, setFailed] = useState(false);
  const safeSrc = safeExternalUrl(src, ["http:", "https:"]);
  if (!safeSrc || failed) {
    return (
      <span
        className="inline-flex max-w-full items-center rounded-md border border-dashed border-line-soft bg-surface-subtle px-2 py-1 text-sm text-ink-muted"
        title={src}
      >
        {alt || "Image unavailable"}
      </span>
    );
  }

  return (
    <img
      alt={alt}
      className="my-2 max-h-80 max-w-full rounded-md border border-line-soft object-contain"
      onError={() => setFailed(true)}
      src={safeSrc}
      title={title}
    />
  );
}

function safeExternalUrl(value: string, allowedProtocols: string[]) {
  try {
    const url = new URL(value);
    return allowedProtocols.includes(url.protocol) ? value : null;
  }
  catch {
    return null;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isStoredArtifact(value: unknown): value is StoredArtifact {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.workspaceId === "string"
    && typeof value.title === "string"
    && typeof value.artifactType === "string"
    && typeof value.createdAt === "number"
    && typeof value.updatedAt === "number";
}

function isStoredRun(value: unknown): value is StoredRun {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.threadId === "string"
    && typeof value.status === "string"
    && typeof value.createdAt === "number"
    && typeof value.updatedAt === "number";
}

function isStoredApproval(value: unknown): value is StoredApprovalRequest {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.threadId === "string"
    && typeof value.kind === "string"
    && typeof value.status === "string"
    && typeof value.title === "string";
}

function isStoredReview(value: unknown): value is StoredReviewChangeset {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.threadId === "string"
    && typeof value.title === "string"
    && typeof value.status === "string"
    && typeof value.filesChanged === "number";
}

function isStoredResearch(value: unknown): value is StoredResearchResource {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.collectionId === "string"
    && typeof value.workspaceId === "string"
    && typeof value.title === "string"
    && typeof value.resourceType === "string";
}

function isStoredTool(value: unknown): value is StoredToolCall {
  return isRecord(value)
    && typeof value.id === "string"
    && typeof value.runId === "string"
    && typeof value.name === "string"
    && typeof value.kind === "string"
    && typeof value.status === "string";
}
