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
import { useMemo } from "react";
import { referenceKey } from "./futureMarkdownTypes";
import { useFutureReference, useFutureReferences } from "./futureReferenceStore";
import { parseFutureMarkdown } from "./parseFutureMarkdown";
import { ArtifactEmbed } from "./renderers/ArtifactEmbed";
import { MissingReference } from "./renderers/MissingReference";
import { ApprovalEmbed, ResearchEmbed, ReviewEmbed, ToolEmbed } from "./renderers/ObjectEmbed";
import { ReferenceChip } from "./renderers/ReferenceChip";
import { RunEmbed } from "./renderers/RunEmbed";

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
          {withStableKeys(node.items, key, item => `${item.checked ?? "plain"}:${inlineSignature(item.children)}`).map(({ item, key: itemKey }) => (
            <li className="pl-1" key={itemKey}>
              <ListItemContent item={item} itemKey={itemKey} workspaceId={workspaceId} />
            </li>
          ))}
        </Tag>
      );
    }
    case "code":
      return (
        <pre className="overflow-auto rounded-lg bg-slate-100 p-3 text-xs leading-5 text-ink" key={key}>
          {node.language ? <div className="mb-2 text-[11px] text-ink-muted">{node.language}</div> : null}
          <code>{node.code}</code>
        </pre>
      );
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
                {withStableKeys(node.headers, `${key}:h`, inlineSignature).map(({ item: header, key: headerKey, ordinal: column }) => (
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
              {withStableKeys(node.rows, `${key}:r`, row => row.map(inlineSignature).join("|")).map(({ item: row, key: rowKey }) => (
                <tr className="odd:bg-surface even:bg-surface-subtle/60" key={rowKey}>
                  {withStableKeys(row, `${rowKey}:c`, inlineSignature).map(({ item: cell, key: cellKey, ordinal: column }) => (
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
          <code className="rounded bg-slate-100 px-1 py-0.5 text-[0.92em] text-ink" key={key}>
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

function withStableKeys<T>(items: T[], seed: string, signature: (item: T) => string) {
  const seen = new Map<string, number>();
  return items.map((item, ordinal) => {
    const base = stableKeyPart(signature(item));
    const count = seen.get(base) ?? 0;
    seen.set(base, count + 1);
    return {
      item,
      key: `${seed}:${base}:${count}`,
      ordinal,
    };
  });
}

function stableKeyPart(value: string) {
  let hash = 5381;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 33) ^ value.charCodeAt(index);
  }
  return (hash >>> 0).toString(36);
}

function inlineSignature(nodes: InlineNode[]): string {
  return nodes.map((node) => {
    if (node.type === "futureReference")
      return `future:${referenceKey(node.reference)}:${node.reference.view}:${node.reference.label ?? ""}`;
    if (node.type === "strong" || node.type === "italic" || node.type === "delete")
      return `${node.type}:${inlineSignature(node.children)}`;
    if (node.type === "link")
      return `link:${node.href}:${inlineSignature(node.children)}`;
    if (node.type === "image")
      return `image:${node.src}:${node.alt}:${node.title ?? ""}`;
    if (node.type === "code")
      return `code:${node.code}`;
    if (node.type === "break")
      return "break";
    return `${node.type}:${node.text}`;
  }).join("\u001F");
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
      rel="noreferrer"
      target="_blank"
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
  const safeSrc = safeExternalUrl(src, ["http:", "https:"]);
  if (!safeSrc) {
    return <span className="text-sm text-ink-muted" title={src}>{alt || "Image omitted"}</span>;
  }

  return (
    <img
      alt={alt}
      className="my-2 max-h-80 max-w-full rounded-md border border-line-soft object-contain"
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
