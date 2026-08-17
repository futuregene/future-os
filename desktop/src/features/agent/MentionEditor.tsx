import type { ClipboardEvent as ReactClipboardEvent, KeyboardEvent as ReactKeyboardEvent, Ref } from "react";
import type { WorkspaceFileResult } from "../../integrations/storage/threadStore";
import { Blocks, FileText } from "lucide-react";
import { useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { searchWorkspaceFiles } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { parseMentionSegments } from "./mentionMarkdown";

/** A skill offered by the `/` menu; `name` is the English slash-command name. */
export interface SkillMentionOption {
  name: string;
  /** Description in the current UI language. */
  description: string;
  /** Chinese name/description, matched by the `/` search but never displayed. */
  nameZh?: string | null;
  descriptionZh?: string | null;
}

export interface MentionEditorHandle {
  /** Serialize to markdown: text verbatim, each file pill → `[name](./path)`. */
  getContent: () => string;
  /** Empty the editor. */
  clear: () => void;
  focus: () => void;
  /**
   * Insert a file mention pill programmatically (e.g. from the file tree),
   * without an active `@` query. `path` is workspace-relative; the pill lands at
   * the caret when it's inside the editor, otherwise appended at the end.
   */
  insertMention: (file: { path: string; name: string }) => void;
  /**
   * Rebuild the editor from a `getContent()` markdown string (text + mention
   * pills); empty string clears it. Inverse of `getContent()`; does not fire
   * `onChange`. Used to restore a persisted draft.
   */
  restore: (content: string) => void;
}

interface MentionEditorProps {
  workspaceId?: string | null;
  /** Installed skills for the `/` menu; omit/empty to disable the menu. */
  skills?: SkillMentionOption[];
  disabled?: boolean;
  placeholder: string;
  className?: string;
  /** Enter pressed (not Shift, not during IME): parent reads getContent, sends, clears. */
  onSubmit: () => void;
  /** Fires when the editor transitions between empty and non-empty. */
  onEmptyChange?: (empty: boolean) => void;
  /**
   * Fires after any user-driven content edit (typing, mention insert, newline,
   * paste) so the parent can persist a draft. NOT fired by `restore()`.
   */
  onChange?: () => void;
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
  skills,
  disabled,
  placeholder,
  className,
  onSubmit,
  onEmptyChange,
  onChange,
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
  // `/` skill trigger: null → inactive; "" → bare `/` (full skill list).
  const [slashQuery, setSlashQuery] = useState<string | null>(null);
  const [selectedSkill, setSelectedSkill] = useState(0);
  const [empty, setEmpty] = useState(true);

  // Skills matching the active `/` query (EN/ZH name + description substring).
  const filteredSkills = useMemo(() => {
    if (slashQuery === null || !skills || skills.length === 0)
      return [];
    const needle = slashQuery.toLowerCase();
    return skills
      .filter(skill =>
        skill.name.toLowerCase().includes(needle)
        || skill.description.toLowerCase().includes(needle)
        || (skill.nameZh ?? "").toLowerCase().includes(needle)
        || (skill.descriptionZh ?? "").toLowerCase().includes(needle))
      .slice(0, 20);
  }, [slashQuery, skills]);
  const skillMenuOpen = slashQuery !== null && !!skills && skills.length > 0;

  // Live mirror of the skills prop so the imperative restore() can rebuild
  // skill pills from `/name` tokens without re-declaring the handle.
  const skillsRef = useRef(skills);
  skillsRef.current = skills;

  useImperativeHandle(ref, () => ({
    getContent: () => serialize(editorRef.current),
    clear: () => {
      if (editorRef.current)
        editorRef.current.innerHTML = "";
      closeMenu();
      syncEmpty();
    },
    focus: () => editorRef.current?.focus(),
    insertMention,
    // Rebuild the DOM from markdown: verbatim text becomes text nodes, each
    // `[name](./path)` mention becomes an atomic pill (same builder as a live
    // pick), so the editor never re-hydrates raw markup.
    restore: (content: string) => {
      const editor = editorRef.current;
      if (editor) {
        editor.innerHTML = "";
        for (const segment of content ? parseMentionSegments(content) : []) {
          if (segment.mention && segment.path)
            editor.appendChild(buildPill({ name: segment.text, path: segment.path.replace(/^\.\//, "") }));
          else if (segment.text)
            appendTextWithSkillPills(editor, segment.text);
        }
        // Focus and park the caret at the end: a bare focus() drops the caret
        // at the start, which reads as "cursor jumped before the restored text".
        editor.focus();
        const selection = window.getSelection();
        const range = document.createRange();
        range.selectNodeContents(editor);
        range.collapse(false);
        selection?.removeAllRanges();
        selection?.addRange(range);
      }
      closeMenu();
      syncEmpty();
    },
  }));

  function closeMenu() {
    setQuery(null);
    setOpen(false);
    setSlashQuery(null);
  }

  function syncEmpty() {
    const next = isEditorEmpty(editorRef.current);
    setEmpty((previous) => {
      if (previous !== next)
        onEmptyChange?.(next);
      return next;
    });
  }

  // Refresh the active trigger (`@` file mention or `/` skill) at the caret.
  function updateTrigger() {
    const editor = editorRef.current;
    const mention = mentionContext(editor);
    if (mention) {
      setQuery(mention.query);
      setSlashQuery(null);
      return;
    }
    setQuery(null);
    const slash = slashContext(editor);
    setSlashQuery(slash ? slash.query : null);
  }

  // Reset the highlighted skill whenever the `/` query changes.
  useEffect(() => {
    setSelectedSkill(0);
  }, [slashQuery]);

  // Debounced workspace-file search driven by the active-mention query.
  useEffect(() => {
    if (query === null || !workspaceId || disabled) {
      setOpen(false);
      setResults([]);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      searchWorkspaceFiles({ limit: 10, query, workspaceId })
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

  // Build the atomic (contenteditable=false) pill span for a file mention. Its
  // `data-path` holds the `./relative` target that serialize() reads back.
  function buildPill(file: { path: string; name: string }): HTMLSpanElement {
    const pill = document.createElement("span");
    pill.setAttribute(PILL_ATTR, "file");
    pill.setAttribute("data-path", `./${file.path}`);
    pill.setAttribute("contenteditable", "false");
    pill.className = "text-accent";
    pill.textContent = file.name;
    return pill;
  }

  // Skill pill: same atomic/highlighted treatment as a file pill; serialize()
  // reads `data-skill` back into the `/name` token the agent understands.
  function buildSkillPill(name: string): HTMLSpanElement {
    const pill = document.createElement("span");
    pill.setAttribute(PILL_ATTR, "skill");
    pill.setAttribute("data-skill", name);
    pill.setAttribute("contenteditable", "false");
    pill.className = "text-accent";
    pill.textContent = `/${name}`;
    return pill;
  }

  // Append plain text to the editor, upgrading every `/name` token that names
  // an installed skill into a pill (used by draft restore, where the serialized
  // markdown only carries the raw token).
  function appendTextWithSkillPills(editor: HTMLDivElement, text: string) {
    const names = new Set((skillsRef.current ?? []).map(skill => skill.name));
    if (names.size === 0) {
      editor.appendChild(document.createTextNode(text));
      return;
    }
    const token = /\/(\w[\w-]*)/g;
    let last = 0;
    for (let match = token.exec(text); match !== null; match = token.exec(text)) {
      const name = match[1]!;
      if (!names.has(name))
        continue;
      if (match.index > last)
        editor.appendChild(document.createTextNode(text.slice(last, match.index)));
      editor.appendChild(buildSkillPill(name));
      last = match.index + match[0].length;
    }
    if (last < text.length)
      editor.appendChild(document.createTextNode(text.slice(last)));
  }

  // Drop a pill (followed by an editable space) at `range`, then place the caret
  // after the space so it rests outside the atomic pill.
  function placePill(range: Range, pill: HTMLSpanElement) {
    const gap = document.createTextNode(" ");
    range.insertNode(gap);
    range.insertNode(pill);

    const after = document.createRange();
    after.setStartAfter(gap);
    after.collapse(true);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(after);

    closeMenu();
    editorRef.current?.focus();
    syncEmpty();
    onChange?.();
  }

  function insertFile(file: WorkspaceFileResult) {
    const editor = editorRef.current;
    const context = mentionContext(editor);
    if (!editor || !context)
      return;

    // Replace the typed `@query` with the pill.
    const range = document.createRange();
    range.setStart(context.textNode, context.atOffset);
    range.setEnd(context.textNode, context.caretOffset);
    range.deleteContents();
    placePill(range, buildPill(file));
  }

  // Replace the typed `/query` with an atomic, highlighted `/name` pill (same
  // treatment as a file pill; serialize() turns it back into the raw token).
  function insertSkill(skill: SkillMentionOption) {
    const editor = editorRef.current;
    const context = slashContext(editor);
    if (!editor || !context)
      return;

    const range = document.createRange();
    range.setStart(context.textNode, context.slashOffset);
    range.setEnd(context.textNode, context.caretOffset);
    range.deleteContents();
    placePill(range, buildSkillPill(skill.name));
  }

  function insertMention(file: { path: string; name: string }) {
    const editor = editorRef.current;
    if (!editor)
      return;
    editor.focus();

    const selection = window.getSelection();
    const caret = selection && selection.rangeCount > 0 ? selection.getRangeAt(0) : null;
    let range: Range;
    if (caret && editor.contains(caret.commonAncestorContainer)) {
      // Caret is inside the editor — insert there.
      range = caret;
      range.deleteContents();
    }
    else {
      // No caret in the editor — append at the end, with a leading space so the
      // pill doesn't butt against existing text.
      range = document.createRange();
      range.selectNodeContents(editor);
      range.collapse(false);
      if (!isEditorEmpty(editor)) {
        const lead = document.createTextNode(" ");
        range.insertNode(lead);
        range.setStartAfter(lead);
        range.collapse(true);
      }
    }
    placePill(range, buildPill(file));
  }

  function insertNewline() {
    const selection = window.getSelection();
    if (!selection || selection.rangeCount === 0)
      return;
    const range = selection.getRangeAt(0);
    range.deleteContents();
    const newline = document.createTextNode("\n");
    range.insertNode(newline);
    if (!newline.nextSibling) {
      // A bare trailing "\n" doesn't grow the box — the empty last line
      // collapses. Park a zero-width space after it (with the caret before it)
      // so the new line renders; serialize() strips ZWSPs on submit.
      const pad = document.createTextNode("\u200B");
      newline.parentNode?.insertBefore(pad, null);
      range.setStartBefore(pad);
    }
    else {
      range.setStartAfter(newline);
    }
    range.collapse(true);
    selection.removeAllRanges();
    selection.addRange(range);
    syncEmpty();
    onChange?.();
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
    if (skillMenuOpen && filteredSkills.length > 0) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setSelectedSkill(index => (index + 1) % filteredSkills.length);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setSelectedSkill(index => (index - 1 + filteredSkills.length) % filteredSkills.length);
        return;
      }
      if (event.key === "Enter" || event.key === "Tab") {
        event.preventDefault();
        const skill = filteredSkills[selectedSkill];
        if (skill)
          insertSkill(skill);
        return;
      }
    }
    if (event.key === "Escape" && (open || skillMenuOpen)) {
      event.preventDefault();
      closeMenu();
      return;
    }

    if (event.key !== "Enter")
      return;
    event.preventDefault();
    if (event.shiftKey || event.ctrlKey) {
      insertNewline(); // Shift+Enter / Ctrl+Enter → literal newline (whitespace-pre-wrap renders it)
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
    updateTrigger();
    syncEmpty();
    onChange?.();
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
      {skillMenuOpen
        ? (
            <SkillMenu
              skills={filteredSkills}
              selectedIndex={selectedSkill}
              emptyLabel={t("composer.noSkillMatches")}
              onSelect={insertSkill}
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
            updateTrigger();
          onChange?.();
        }}
        onKeyDown={handleKeyDown}
        onPaste={handlePaste}
        onCompositionStart={() => { isComposingRef.current = true; }}
        onCompositionEnd={() => {
          isComposingRef.current = false;
          // After the composed text lands, re-check for an active `@`/`/` trigger.
          requestAnimationFrame(() => {
            if (!isComposingRef.current) {
              updateTrigger();
              syncEmpty();
              onChange?.();
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

function SkillMenu({
  emptyLabel,
  onSelect,
  selectedIndex,
  skills,
}: {
  emptyLabel: string;
  onSelect: (skill: SkillMentionOption) => void;
  selectedIndex: number;
  skills: SkillMentionOption[];
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
      {skills.length === 0
        ? <div className="px-2 py-2 text-sm text-ink-muted">{emptyLabel}</div>
        : null}
      {skills.map((skill, index) => (
        <button
          className={cn(
            "flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left transition-colors",
            index === selectedIndex ? "bg-surface-subtle" : "hover:bg-surface-subtle",
          )}
          data-menu-index={index}
          key={skill.name}
          onMouseDown={(event) => {
            // Keep the editor's selection/focus so insertion targets the caret.
            event.preventDefault();
            onSelect(skill);
          }}
          type="button"
        >
          <Blocks className="mt-0.5 size-4 shrink-0 text-ink-soft" />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm font-medium text-ink">
              /
              {skill.name}
            </span>
            {skill.description
              ? <span className="block truncate text-xs text-ink-muted">{skill.description}</span>
              : null}
          </span>
        </button>
      ))}
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
 * The `@` may appear anywhere in the text (start, after whitespace, or inside
 * CJK sentences); an ASCII letter/digit or `./@` right before it suppresses the
 * trigger so emails (`foo@bar`) and paths (`./x`, `a/b`) never open the menu.
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
  const match = before.match(/(^|[^\w.@/])@([^\s@]*)$/);
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

/**
 * The active `/` skill trigger at the caret, if any. Same caret scan as
 * `mentionContext`, but matches `/query`. Like `@`, the `/` may sit anywhere
 * in the text (including mid-sentence in CJK); an ASCII letter/digit or `./@`
 * right before it suppresses the trigger so paths (`./x`, `a/b`) and dates
 * (`2026/07`) never open the skill menu.
 */
function slashContext(editor: HTMLDivElement | null): {
  query: string;
  textNode: Text;
  slashOffset: number;
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
  const match = before.match(/(^|[^\w.@/])\/([^\s/]*)$/);
  if (!match)
    return null;
  const query = match[2] ?? "";
  return {
    query,
    textNode: node as Text,
    slashOffset: caretOffset - query.length - 1, // index of `/`
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
    const pillKind = element.getAttribute(PILL_ATTR);
    if (pillKind === "skill") {
      out += `/${element.getAttribute("data-skill") ?? ""}`;
      return;
    }
    if (pillKind) {
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
