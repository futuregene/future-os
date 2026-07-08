import type { ClipboardEvent as ReactClipboardEvent, KeyboardEvent as ReactKeyboardEvent, Ref } from "react";
import type { WorkspaceFileResult } from "../../integrations/storage/threadStore";
import { FileText } from "lucide-react";
import { useEffect, useImperativeHandle, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { searchWorkspaceFiles } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";

export interface MentionEditorHandle {
  /** Serialize to markdown: text verbatim, each file pill → `[name](./path)`. */
  getContent: () => string;
  /** Empty the editor. */
  clear: () => void;
  focus: () => void;
}

interface MentionEditorProps {
  workspaceId?: string | null;
  disabled?: boolean;
  placeholder: string;
  className?: string;
  /** Enter pressed (not Shift, not during IME): parent reads getContent, sends, clears. */
  onSubmit: () => void;
  /** Fires when the editor transitions between empty and non-empty. */
  onEmptyChange?: (empty: boolean) => void;
  /** Pasted image files, handed to the parent to attach. */
  onPasteImages?: (files: File[]) => void;
  ref?: Ref<MentionEditorHandle>;
}

/** Marks a file pill span; `data-path` holds the `./relative/path` target. */
const PILL_ATTR = "data-mention";

/**
 * `@`-mention editor. A non-controlled `contentEditable` div: React renders it
 * empty once and never re-renders its contents — all edits are imperative DOM
 * ops. This is the only way `contentEditable` coexists with IME in WebKit (our
 * Tauri macOS webview), which cancels a composition if the DOM is mutated under
 * it. File picks become `contentEditable=false` pill spans (native atomic
 * delete); on submit the DOM serializes back to `[name](./path)` markdown — the
 * exact format the plain textarea produced before.
 */
export function MentionEditor({
  workspaceId,
  disabled,
  placeholder,
  className,
  onSubmit,
  onEmptyChange,
  onPasteImages,
  ref,
}: MentionEditorProps) {
  const { t } = useTranslation("agent");
  const editorRef = useRef<HTMLDivElement | null>(null);
  const isComposingRef = useRef(false);
  // null → no active mention; "" → `@` with empty query (recents).
  const [query, setQuery] = useState<string | null>(null);
  const [results, setResults] = useState<WorkspaceFileResult[]>([]);
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState(0);
  const [empty, setEmpty] = useState(true);

  useImperativeHandle(ref, () => ({
    getContent: () => serialize(editorRef.current),
    clear: () => {
      if (editorRef.current)
        editorRef.current.innerHTML = "";
      closeMenu();
      syncEmpty();
    },
    focus: () => editorRef.current?.focus(),
  }));

  function closeMenu() {
    setQuery(null);
    setOpen(false);
  }

  function syncEmpty() {
    const next = isEditorEmpty(editorRef.current);
    setEmpty((previous) => {
      if (previous !== next)
        onEmptyChange?.(next);
      return next;
    });
  }

  // Refresh the active-mention query from the current caret position.
  function updateMention() {
    const context = mentionContext(editorRef.current);
    setQuery(context ? context.query : null);
  }

  // Debounced workspace-file search driven by the active-mention query.
  useEffect(() => {
    if (query === null || !workspaceId || disabled) {
      setOpen(false);
      setResults([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      searchWorkspaceFiles({ limit: 20, query, workspaceId })
        .then((next) => {
          if (!cancelled) {
            setResults(next);
            setSelected(0);
            setOpen(true);
          }
        })
        .catch(() => {
          if (!cancelled) {
            setResults([]);
            setOpen(false);
          }
        });
    }, 120);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query, workspaceId, disabled]);

  function insertFile(file: WorkspaceFileResult) {
    const editor = editorRef.current;
    const selection = window.getSelection();
    const context = mentionContext(editor);
    if (!editor || !selection || !context)
      return;

    // Replace the typed `@query` with the pill.
    const range = document.createRange();
    range.setStart(context.textNode, context.atOffset);
    range.setEnd(context.textNode, context.caretOffset);
    range.deleteContents();

    const pill = document.createElement("span");
    pill.setAttribute(PILL_ATTR, "file");
    pill.setAttribute("data-path", `./${file.path}`);
    pill.setAttribute("contenteditable", "false");
    pill.className = "text-accent";
    pill.textContent = file.name;

    // Insert a trailing space first, then the pill before it, so the caret can
    // rest after the (editable) space rather than inside the atomic pill.
    const gap = document.createTextNode(" ");
    range.insertNode(gap);
    range.insertNode(pill);

    const after = document.createRange();
    after.setStartAfter(gap);
    after.collapse(true);
    selection.removeAllRanges();
    selection.addRange(after);

    closeMenu();
    editor.focus();
    syncEmpty();
  }

  function insertNewline() {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0)
      return;
    const range = selection.getRangeAt(0);
    range.deleteContents();
    const newline = document.createTextNode("\n");
    range.insertNode(newline);
    range.setStartAfter(newline);
    range.collapse(true);
    selection.removeAllRanges();
    selection.addRange(range);
    syncEmpty();
  }

  function handleKeyDown(event: ReactKeyboardEvent<HTMLDivElement>) {
    // Hand every keystroke to the IME while composing (Enter commits, arrows
    // pick candidates). keyCode 229 covers webviews that leave isComposing unset
    // on the committing keydown. Mirrors the old textarea guard.
    if (event.nativeEvent.isComposing || isComposingRef.current || event.nativeEvent.keyCode === 229)
      return;

    if (open && results.length > 0) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setSelected(index => (index + 1) % results.length);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setSelected(index => (index - 1 + results.length) % results.length);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        const file = results[selected];
        if (file)
          insertFile(file);
        return;
      }
    }
    if (event.key === "Escape" && open) {
      event.preventDefault();
      closeMenu();
      return;
    }

    if (event.key !== "Enter")
      return;
    event.preventDefault();
    if (event.shiftKey) {
      insertNewline(); // Shift+Enter → literal newline (whitespace-pre-wrap renders it)
      return;
    }
    onSubmit();
  }

  function handlePaste(event: ReactClipboardEvent<HTMLDivElement>) {
    // Pasted images become attachments (handled by the parent), never editor text.
    const imageFiles = Array.from(event.clipboardData.items)
      .filter(item => item.kind === "file" && item.type.startsWith("image/"))
      .map(item => item.getAsFile())
      .filter((file): file is File => file !== null);
    if (imageFiles.length > 0) {
      event.preventDefault();
      onPasteImages?.(imageFiles);
      return;
    }

    // Otherwise force plain text so pasted rich HTML can't smuggle markup or
    // block nodes into the editor.
    const text = event.clipboardData.getData("text/plain");
    event.preventDefault();
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0)
      return;
    const range = selection.getRangeAt(0);
    range.deleteContents();
    const node = document.createTextNode(text);
    range.insertNode(node);
    range.setStartAfter(node);
    range.collapse(true);
    selection.removeAllRanges();
    selection.addRange(range);
    updateMention();
    syncEmpty();
  }

  return (
    <div className="relative">
      {open && query !== null
        ? (
            <FileMenu
              results={results}
              selectedIndex={selected}
              emptyLabel={t("composer.noFiles")}
              onSelect={insertFile}
            />
          )
        : null}
      {empty
        ? (
            <div className="pointer-events-none absolute left-2 top-1 select-none text-sm leading-5 text-ink-muted">
              {placeholder}
            </div>
          )
        : null}
      <div
        ref={editorRef}
        role="textbox"
        aria-multiline="true"
        aria-label={placeholder}
        contentEditable={!disabled}
        suppressContentEditableWarning
        className={cn(
          "max-h-[40vh] min-h-14 w-full overflow-y-auto whitespace-pre-wrap break-words px-2 py-1 text-sm leading-5 text-ink outline-none",
          className,
        )}
        onInput={() => {
          syncEmpty();
          if (!isComposingRef.current)
            updateMention();
        }}
        onKeyDown={handleKeyDown}
        onPaste={handlePaste}
        onCompositionStart={() => { isComposingRef.current = true; }}
        onCompositionEnd={() => {
          isComposingRef.current = false;
          // After the composed text lands, re-check for an active `@` mention.
          requestAnimationFrame(() => {
            if (!isComposingRef.current) {
              updateMention();
              syncEmpty();
            }
          });
        }}
      />
    </div>
  );
}

function FileMenu({
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
  return (
    <div className="absolute bottom-full left-2 z-30 mb-2 w-[min(30rem,calc(100%-1rem))] rounded-lg border border-line-soft bg-surface p-1 shadow-panel">
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

/** True when the editor has no text and no pills. */
function isEditorEmpty(editor: HTMLDivElement | null): boolean {
  if (!editor)
    return true;
  if (editor.querySelector(`[${PILL_ATTR}]`))
    return false;
  return (editor.textContent ?? "").trim().length === 0;
}

/**
 * The active `@` mention at the caret, if any. Reads the caret's text node and
 * matches `@query` at its end — a pill (separate node) naturally bounds it.
 */
function mentionContext(editor: HTMLDivElement | null): {
  query: string;
  textNode: Text;
  atOffset: number;
  caretOffset: number;
} | null {
  const selection = window.getSelection();
  if (!editor || !selection || selection.rangeCount === 0 || !selection.isCollapsed)
    return null;
  const node = selection.anchorNode;
  if (!node || node.nodeType !== Node.TEXT_NODE || !editor.contains(node))
    return null;
  const caretOffset = selection.anchorOffset;
  const before = (node.textContent ?? "").slice(0, caretOffset);
  const match = before.match(/(^|\s)@([^\s@]*)$/);
  if (!match)
    return null;
  const query = match[2] ?? "";
  return {
    query,
    textNode: node as Text,
    atOffset: caretOffset - query.length - 1, // index of `@`
    caretOffset,
  };
}

/** Serialize the editor: text verbatim, pills → `[name](./path)` markdown links. */
function serialize(editor: HTMLDivElement | null): string {
  if (!editor)
    return "";
  let out = "";
  const visit = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      out += node.textContent ?? "";
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE)
      return;
    const element = node as HTMLElement;
    if (element.getAttribute(PILL_ATTR)) {
      const label = (element.textContent ?? "").replace(/\[/g, "(").replace(/\]/g, ")");
      const path = element.getAttribute("data-path") ?? "";
      // Angle-wrap whenever the path holds whitespace OR parens: a bare `)` in
      // the path closes the markdown link early, truncating downstream parsing
      // (MessageBlock's MENTION_LINK matches the `<...>` form for these).
      out += `[${label}](${/[\s()]/.test(path) ? `<${path}>` : path})`;
      return;
    }
    if (element.tagName === "BR") {
      out += "\n";
      return;
    }
    for (const child of Array.from(element.childNodes))
      visit(child);
    // A browser-inserted block wrapper implies a line break after it.
    if (element.tagName === "DIV" || element.tagName === "P")
      out += "\n";
  };
  for (const child of Array.from(editor.childNodes))
    visit(child);
  return out.replace(/\u200B/g, ""); // strip any stray zero-width spaces
}
