import type { WorkspaceFileResult } from "../../../integrations/storage/threadStore";
import type { ContextToolOption, SkillMentionOption, SlashMenuItem } from "../MentionEditor";
import type { SessionMentionGroup, SessionMentionOption } from "../sessionMention";
import { Blocks, FileText, MessagesSquare, Minimize2 } from "lucide-react";
import { useEffect, useRef } from "react";
import { cn } from "../../../lib/cn";
import { hasMixedSlashResults } from "../slashMenu";

export function FileMenu({
  emptyLabel,
  onSelect,
  results,
  selectedIndex,
}: {
  emptyLabel: string;
  onSelect: (file: WorkspaceFileResult) => void;
  results: WorkspaceFileResult[];
  selectedIndex: number;
}) {
  // Keep the keyboard-highlighted row visible while the list scrolls.
  const listRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-menu-index="${selectedIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  return (
    <div ref={listRef} className="absolute bottom-full left-2 z-30 mb-2 max-h-72 w-[min(30rem,calc(100%-1rem))] overflow-y-auto rounded-lg border border-line-soft bg-surface p-1 shadow-panel">
      {results.length === 0
        ? <div className="px-2 py-2 text-sm text-ink-muted">{emptyLabel}</div>
        : null}
      {results.map((file, index) => {
        const dir = file.path.slice(0, file.path.length - file.name.length);
        return (
          <button
            className={cn(
              "flex h-9 w-full items-center gap-2 rounded-md px-2 text-left transition-colors",
              index === selectedIndex ? "bg-surface-subtle" : "hover:bg-surface-subtle",
            )}
            data-menu-index={index}
            key={file.path}
            onMouseDown={(event) => {
              // Keep the editor's selection/focus so insertion targets the caret.
              event.preventDefault();
              onSelect(file);
            }}
            type="button"
          >
            <FileText className="size-4 shrink-0 text-ink-soft" />
            <span className="min-w-0 flex-1 truncate text-sm">
              {dir ? <span className="text-ink-muted">{dir}</span> : null}
              <span className="font-medium text-ink">{file.name}</span>
            </span>
          </button>
        );
      })}
    </div>
  );
}

export function SlashMenu({
  emptyLabel,
  groups,
  onSelect,
  selectedIndex,
  skillsLabel,
}: {
  emptyLabel: string;
  groups: { contextTools: ContextToolOption[]; skills: SkillMentionOption[] };
  onSelect: (item: SlashMenuItem) => void;
  selectedIndex: number;
  skillsLabel: string;
}) {
  // Keep the keyboard-highlighted row visible while the list scrolls.
  const listRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-menu-index="${selectedIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  const items: SlashMenuItem[] = [
    ...groups.contextTools.map(tool => ({ kind: "context-tool" as const, tool })),
    ...groups.skills.map(skill => ({ kind: "skill" as const, skill })),
  ];
  const mixed = hasMixedSlashResults(groups);
  return (
    <div ref={listRef} className="absolute bottom-full left-2 z-30 mb-2 max-h-72 w-[min(30rem,calc(100%-1rem))] overflow-y-auto rounded-lg border border-line-soft bg-surface p-1 shadow-panel">
      {items.length === 0
        ? <div className="px-2 py-2 text-sm text-ink-muted">{emptyLabel}</div>
        : null}
      {items.map((item, index) => (
        <div key={item.kind === "context-tool" ? `tool:${item.tool.id}` : `skill:${item.skill.name}`}>
          {mixed && index === groups.contextTools.length
            ? <div className="px-2 pb-1 pt-2 text-xs font-medium text-ink-muted">{skillsLabel}</div>
            : null}
          <button
            className={cn(
              "flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left transition-colors",
              index === selectedIndex ? "bg-surface-subtle" : "hover:bg-surface-subtle",
            )}
            data-menu-index={index}
            onMouseDown={(event) => {
              // Keep the editor's selection/focus so insertion targets the caret.
              event.preventDefault();
              onSelect(item);
            }}
            type="button"
          >
            {item.kind === "context-tool"
              ? <Minimize2 className="mt-0.5 size-4 shrink-0 text-ink-soft" />
              : <Blocks className="mt-0.5 size-4 shrink-0 text-ink-soft" />}
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-medium text-ink">
                {item.kind === "context-tool" ? item.tool.name : `/${item.skill.name}`}
              </span>
              {(item.kind === "context-tool" ? item.tool.description : item.skill.description)
                ? <span className="block truncate text-xs text-ink-muted">{item.kind === "context-tool" ? item.tool.description : item.skill.description}</span>
                : null}
            </span>
          </button>
        </div>
      ))}
    </div>
  );
}

/**
 * The `#` conversation menu: results grouped by workspace (chats first), with a
 * heading per group. `items` is the flattened, navigable order that
 * `selectedIndex` indexes into — headings are not rows.
 */
export function SessionMenu({
  chatsLabel,
  emptyLabel,
  groups,
  items,
  onSelect,
  selectedIndex,
  workspaceLabel,
}: {
  chatsLabel: string;
  emptyLabel: string;
  groups: SessionMentionGroup[];
  items: SessionMentionOption[];
  onSelect: (session: SessionMentionOption) => void;
  selectedIndex: number;
  /** Heading for a workspace group the store has no name for (deleted row). */
  workspaceLabel: string;
}) {
  // Keep the keyboard-highlighted row visible while the list scrolls.
  const listRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    listRef.current
      ?.querySelector(`[data-menu-index="${selectedIndex}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  // A lone chats group needs no heading; any workspace group does (it names the
  // workspace the conversations below it came from).
  const headings = groups.length > 1 || Boolean(groups[0]?.workspace);
  const indexOf = new Map(items.map((item, index) => [item.sessionId, index]));
  return (
    <div ref={listRef} className="absolute bottom-full left-2 z-30 mb-2 max-h-72 w-[min(30rem,calc(100%-1rem))] overflow-y-auto rounded-lg border border-line-soft bg-surface p-1 shadow-panel">
      {items.length === 0
        ? <div className="px-2 py-2 text-sm text-ink-muted">{emptyLabel}</div>
        : null}
      {groups.map((group) => {
        return (
          <div key={group.workspace?.id ?? "__chats"}>
            {headings
              ? (
                  <div className="truncate px-2 pb-1 pt-2 text-xs font-medium text-ink-muted">
                    {group.workspace
                      ? (group.workspace.name || workspaceLabel)
                      : chatsLabel}
                  </div>
                )
              : null}
            {group.sessions.map((session) => {
              const index = indexOf.get(session.sessionId) ?? 0;
              return (
                <button
                  className={cn(
                    "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors",
                    index === selectedIndex ? "bg-surface-subtle" : "hover:bg-surface-subtle",
                  )}
                  data-menu-index={index}
                  key={session.sessionId}
                  onMouseDown={(event) => {
                    // Keep the editor's selection/focus so insertion targets the caret.
                    event.preventDefault();
                    onSelect(session);
                  }}
                  type="button"
                >
                  <MessagesSquare className="size-4 shrink-0 text-ink-soft" />
                  <span className="min-w-0 flex-1 truncate text-sm text-ink">{session.title}</span>
                </button>
              );
            })}
          </div>
        );
      })}
    </div>
  );
}
