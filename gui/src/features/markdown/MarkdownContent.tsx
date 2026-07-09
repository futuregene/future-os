import type { ReactNode } from "react";
import type { StoredFile } from "../../integrations/storage/types";
import type { FutureReference, InlineNode, MarkdownNode } from "./futureMarkdownTypes";
import { useMemo } from "react";
import { useFutureReference, useFutureReferences } from "./futureReferenceStore";
import { parseFutureMarkdown } from "./parseFutureMarkdown";
import { PreviewMarkdownContext, usePreviewMarkdown } from "./PreviewMarkdownContext";
import { CodeBlock } from "./renderers/CodeBlock";
import { FileLink } from "./renderers/FileLink";
import { renderFileReference } from "./renderers/fileReference";
import { FutureEmbed } from "./renderers/FutureEmbed";
import { PendingReference } from "./renderers/PendingReference";
import { ReferenceChip } from "./renderers/ReferenceChip";
import { SafeImage, SafeLink } from "./renderers/SafeLink";
import { usePreviewLinkPath } from "./usePreviewLinkPath";

interface MarkdownContentProps {
  content: string;
  workspaceId?: string | null;
  /**
   * When set, renders in file-preview mode (see `PreviewMarkdownContext`): this
   * is the absolute path of the previewed file, used to resolve relative links.
   * The chat stream omits it, keeping its link behavior unchanged.
   */
  basePath?: string;
}

export function MarkdownContent({ content, workspaceId, basePath }: MarkdownContentProps) {
  const document = useMemo(() => parseFutureMarkdown(content), [content]);
  useFutureReferences(workspaceId, document.references);

  const body = <div className="space-y-3">{document.nodes.map((node, index) => renderBlock(node, workspaceId, `b${index}`))}</div>;
  if (basePath)
    return <PreviewMarkdownContext value={{ basePath }}>{body}</PreviewMarkdownContext>;
  return body;
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
        <blockquote className="border-l-2 border-line pl-3 text-ink-soft" key={key}>
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
  const preview = usePreviewMarkdown();
  // In preview mode a file link resolves against the previewed file's directory
  // (there is no workspace root), bypassing the workspace-scoped reference store.
  if (preview && reference.targetType === "file")
    return <PreviewFileReference basePath={preview.basePath} reference={reference} />;
  return <WorkspaceFutureReference reference={reference} workspaceId={workspaceId} />;
}

function WorkspaceFutureReference({
  reference,
  workspaceId,
}: {
  reference: FutureReference;
  workspaceId: string | null | undefined;
}) {
  const resolved = useFutureReference(workspaceId, reference);
  const fileLink = renderFileReference(reference, resolved);
  if (fileLink)
    return fileLink;
  return <ReferenceChip reference={reference} resolved={resolved} />;
}

function PreviewFileReference({
  basePath,
  reference,
}: {
  basePath: string;
  reference: FutureReference;
}) {
  const resolved = usePreviewLinkPath(basePath, reference.targetId);
  if (!resolved)
    return <PendingReference reference={reference} />;
  const file: StoredFile = {
    path: resolved.path,
    name: resolved.name,
    insideWorkspace: false,
    relativePath: null,
  };
  return <FileLink file={file} />;
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
