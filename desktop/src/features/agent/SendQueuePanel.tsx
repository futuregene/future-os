import { Check, ChevronDown, Image as ImageIcon, Paperclip, Pencil, Play, RotateCcw, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { SelectMenu, SelectMenuItem } from "../../components/ui/SelectMenu";
import { cn } from "../../lib/cn";

/**
 * One file a waiting message carries, as the panel draws it.
 *
 * Both queues report this shape — this machine's Agent (a local path) and a
 * paired host's (a path only that host can serve) — so the panel is written
 * against the file, not against the machine it came from.
 */
export interface SendQueueAttachmentItem {
  path: string;
  /** "image" | "file". */
  kind: string;
  name: string;
  /** Byte size, or 0 when the queue did not report one. */
  size: number;
}

/** One row of the panel: a message waiting for the run to finish. */
export interface SendQueuePanelItem {
  id: string;
  /** The text as it will be sent (empty for an attachment-only message). */
  content: string;
  /**
   * The files the message carries.
   *
   * `undefined` when the queue behind this row does not report them (an Agent
   * older than the field, or a client that has not read it yet). One of those is
   * then told, in the editor, that saving sends the words alone — which is why
   * absence is spelled as absence and never as a count of zero. A message with
   * no files reports `[]`.
   */
  attachments?: readonly SendQueueAttachmentItem[];
  /**
   * Parked by the Agent: it waited too long to run by itself and now starts only
   * when the reader says so. Undefined means "will run at the next turn
   * boundary", which is what every row is on a host without that rule.
   */
  parked?: boolean;
  /**
   * The answer a `futureos-choice` card gave this message, when it came from one.
   *
   * `content` already reads `[question → answer]`, so the row says what it will
   * send without this. What only this can do is let the reader change *which*
   * option was picked before the run reads it, because the sentence names the
   * chosen option and drops the ones it could still switch to (design §5.8).
   *
   * `undefined` means the prompt answered no card — also what a queue that
   * predates the field reports, which must not be drawn as an empty chip.
   */
  choice?: SendQueueChoiceItem;
}

/** One queued answer, as the panel re-offers it. */
export interface SendQueueChoiceItem {
  mode: "single" | "multi";
  title: string;
  options: readonly { id: string; label: string }[];
  selectedIds: readonly string[];
}

/** The picked options' labels, in pick order — what the chip prints. */
function choiceLabels(choice: SendQueueChoiceItem): string[] {
  return choice.selectedIds
    .map(id => choice.options.find(option => option.id === id)?.label)
    .filter((label): label is string => Boolean(label));
}

/**
 * A pick on one of the answer's rows: multi toggles, single replaces.
 *
 * The same rule the card and the composer's pill use (`pickChoiceOption`), so
 * changing the answer on a waiting row behaves like changing it anywhere else.
 */
function toggleChoice(choice: SendQueueChoiceItem, optionId: string): SendQueueChoiceItem {
  if (choice.mode === "multi") {
    const picked = choice.selectedIds.includes(optionId)
      ? choice.selectedIds.filter(id => id !== optionId)
      : [...choice.selectedIds, optionId];
    return { ...choice, selectedIds: picked };
  }
  return { ...choice, selectedIds: [optionId] };
}

/**
 * The send queue, above the box that filled it.
 *
 * A queued message is not yet gone, so this is the one place to look at it, fix
 * a typo in it, or take it back before the run it waits for ends. It sits above
 * the composer rather than in the transcript because it is part of what the
 * reader is about to send, not something that was sent — and it disappears the
 * moment the queue empties, because an empty panel would only take space.
 *
 * Shared by this app's composer and the one for a paired computer: the queue is
 * the same promise either way, and only the machine the message leaves from
 * differs.
 */
export function SendQueuePanel({
  attachmentsUnreported,
  failed,
  items,
  onPreviewAttachment,
  onRemove,
  onRetry,
  onRelease,
  onUpdate,
  onUpdateChoice,
}: {
  /**
   * No row's queue reports which files a message carries, so an edit — a removal
   * and a re-send, the only kind there is — sends the new text and leaves those
   * files behind. Said in the editor, where the reader is about to make that
   * trade, rather than as a number the panel cannot know.
   *
   * A row that does report its files overrides this: that row's edit keeps them,
   * and warning about a loss that will not happen would be wrong.
   */
  attachmentsUnreported?: boolean;
  /** The last automatic delivery failed; offer a retry instead of the hint. */
  failed?: boolean;
  items: readonly SendQueuePanelItem[];
  /**
   * Open one file of a waiting message, exactly as the transcript would.
   *
   * Given by the composer that knows where the file is: the same panel draws this
   * machine's queue (a local path to open) and a paired computer's (a path only
   * that host can serve, which has to be fetched first). Absent, the chips are
   * still named but not clickable.
   */
  onPreviewAttachment?: (attachment: SendQueueAttachmentItem) => void;
  /** Start a parked row now. Offered only when the queue can hold one (Agent-side queues). */
  onRelease?: (id: string) => void;
  onRemove: (id: string) => void;
  onRetry?: () => void;
  onUpdate: (id: string, content: string) => void;
  /**
   * Change which option a waiting message's answer picked.
   *
   * Its own callback rather than an extra argument to `onUpdate`: the two are
   * different edits (structured answer vs. prose), they go to different Agent
   * commands, and one signature carrying both would have to branch inside.
   *
   * Absent means this queue's rows are not editable in that way — the chip is
   * then still named, just not clickable.
   */
  onUpdateChoice?: (id: string, choice: SendQueueChoiceItem) => void;
}) {
  const { t } = useTranslation("agent");
  // Only one row is edited at a time — the panel is a list of what is about to
  // leave, not a second editor.
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");
  // Which row's answer menu is open. One at a time, like the editor above: the
  // panel is a list of what is about to leave, not a second form.
  const [choiceOpenId, setChoiceOpenId] = useState<string | null>(null);

  if (items.length === 0)
    return null;

  function startEditing(item: SendQueuePanelItem) {
    setEditingId(item.id);
    setEditingText(item.content);
  }

  function stopEditing() {
    setEditingId(null);
    setEditingText("");
  }

  function saveEditing(id: string) {
    onUpdate(id, editingText);
    stopEditing();
  }

  return (
    <section
      aria-label={t("composer.queue.title")}
      className="rounded-md border border-line-soft bg-surface-subtle/70 px-2.5 py-2"
      data-send-queue=""
    >
      <header className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-baseline gap-2">
          <span className="text-xs font-medium text-ink">{t("composer.queue.title")}</span>
          <span className="text-xs tabular-nums text-ink-muted">
            {t("composer.queue.count", { count: items.length })}
          </span>
        </div>
        {failed
          ? (
              <button
                className="inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-xs font-medium text-warning transition-colors hover:bg-warning-soft"
                data-queue-retry=""
                onClick={onRetry}
                type="button"
              >
                <RotateCcw className="size-3" />
                {t("composer.queue.retry")}
              </button>
            )
          : (
              <span className="truncate text-right text-xs text-ink-muted">
                {/* A parked row runs only when someone presses its own button,
                    so the merge this line describes would never happen to it.
                    With one row there is nothing to merge with: it leaves on
                    its own at the turn boundary, and saying "merged" there
                    promised a combination the queue never performs. */}
                {items.some(item => item.parked)
                  ? t("composer.queue.hintParked")
                  : items.length === 1
                    ? t("composer.queue.hintOne")
                    : t("composer.queue.hint")}
              </span>
            )}
      </header>
      {/* The rows scroll inside the panel, not with the window.
          This block floats over the conversation, so whatever it draws is taken
          from the transcript underneath it — and it used to draw every waiting
          row with no ceiling. A queue long enough to outgrow the window then had
          its top rows clipped away by the frame's own `overflow-hidden`, which
          is not merely unreadable: their edit and delete buttons could not be
          hit at all (the Agent holds up to 128 rows for one conversation, see
          `DEFAULT_SESSION_QUEUE_CAPACITY`). Capped here, the same rows are
          reachable by scrolling the list. The list keeps at most 45% of the
          window and never more than 26rem — past that the rows scroll. */}
      <ol className="mt-1.5 max-h-[min(45vh,26rem)] space-y-1 overflow-y-auto overscroll-contain">
        {items.map((item, index) => {
          const editing = editingId === item.id;
          return (
            <li
              className="flex items-start gap-2 rounded-md bg-surface px-2 py-1.5"
              data-queue-item=""
              key={item.id}
            >
              <span className="mt-0.5 w-4 shrink-0 text-right text-[11px] tabular-nums leading-5 text-ink-muted">
                {index + 1}
              </span>
              {editing
                ? (
                    <div className="min-w-0 flex-1">
                      {attachmentsUnreported && item.attachments === undefined
                        ? (
                            <p className="mb-1 text-[11px] leading-4 text-ink-muted">
                              {t("composer.queue.editDropsFiles")}
                            </p>
                          )
                        : null}
                      <textarea
                        aria-label={t("composer.queue.editLabel")}
                        autoFocus
                        className="block max-h-40 w-full resize-y rounded border border-line bg-surface px-1.5 py-1 text-xs leading-5 text-ink outline-none focus:border-focus"
                        onChange={event => setEditingText(event.target.value)}
                        onKeyDown={(event) => {
                          if (event.key === "Escape") {
                            event.preventDefault();
                            stopEditing();
                          }
                        }}
                        rows={Math.min(6, Math.max(2, editingText.split("\n").length))}
                        value={editingText}
                      />
                      <div className="mt-1 flex items-center justify-end gap-1.5">
                        <button
                          className="rounded px-1.5 py-0.5 text-xs text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink"
                          onClick={stopEditing}
                          type="button"
                        >
                          {t("composer.queue.cancel")}
                        </button>
                        <button
                          className="inline-flex items-center gap-1 rounded bg-accent-fill px-1.5 py-0.5 text-xs font-medium text-white transition-colors hover:bg-accent-fill-hover"
                          data-queue-save=""
                          onClick={() => saveEditing(item.id)}
                          type="button"
                        >
                          <Check className="size-3" />
                          {t("composer.queue.save")}
                        </button>
                      </div>
                    </div>
                  )
                : (
                    <>
                      <div className="min-w-0 flex-1">
                        {item.content
                          ? (
                              <p className="line-clamp-3 whitespace-pre-wrap wrap-break-word text-xs leading-5 text-ink">
                                {item.content}
                              </p>
                            )
                          : (
                              <p className="text-xs leading-5 text-ink-muted italic">
                                {t("composer.queue.attachmentsOnly")}
                              </p>
                            )}
                        {/* The answer this message carries (§5.8.2).
                            The chip is the button — the row already has a
                            pencil for its words, and a second one a few pixels
                            away meaning something else is the ambiguity this
                            design exists to remove. A chevron says "this opens
                            something", which a pencil cannot. */}
                        {item.choice && item.choice.selectedIds.length > 0
                          ? (
                              <SelectMenu
                                align="left"
                                open={choiceOpenId === item.id}
                                onDismiss={() => setChoiceOpenId(null)}
                                panelClassName="w-72 max-h-72 overflow-y-auto"
                                portal
                                trigger={(
                                  <button
                                    aria-label={t("composer.queue.choiceChange", {
                                      title: item.choice.title,
                                      label: choiceLabels(item.choice).join("、"),
                                    })}
                                    className={cn(
                                      "mt-1 inline-flex max-w-full items-center gap-1 rounded border border-line-soft bg-accent-soft/60 px-1.5 py-0.5 text-[11px] leading-4 text-accent-ink",
                                      onUpdateChoice ? "hover:bg-accent-soft" : "cursor-default",
                                    )}
                                    data-queue-choice=""
                                    data-queue-choice-action="change"
                                    disabled={!onUpdateChoice}
                                    onClick={() => setChoiceOpenId(current => current === item.id ? null : item.id)}
                                    title={t("composer.queue.choiceChange", {
                                      title: item.choice.title,
                                      label: choiceLabels(item.choice).join("、"),
                                    })}
                                    type="button"
                                  >
                                    <span className="truncate">{item.choice.title}</span>
                                    <span aria-hidden className="shrink-0 opacity-60">·</span>
                                    <span className="truncate font-medium">
                                      {choiceLabels(item.choice).join("、")}
                                    </span>
                                    <ChevronDown className="size-2.5 shrink-0 opacity-70" />
                                  </button>
                                )}
                              >
                                {item.choice.options.map(option => (
                                  <SelectMenuItem
                                    className="py-1.5"
                                    key={option.id}
                                    onSelect={() => {
                                      setChoiceOpenId(null);
                                      onUpdateChoice?.(item.id, toggleChoice(item.choice!, option.id));
                                    }}
                                    selected={item.choice!.selectedIds.includes(option.id)}
                                  >
                                    {option.label}
                                  </SelectMenuItem>
                                ))}
                                {/* Taking the answer off while keeping the words:
                                    the same exit the card offers, and not a
                                    third button on the chip. */}
                                <SelectMenuItem
                                  className="border-t border-line-soft py-1.5"
                                  onSelect={() => {
                                    setChoiceOpenId(null);
                                    onUpdateChoice?.(item.id, { ...item.choice!, selectedIds: [] });
                                  }}
                                  selected={false}
                                >
                                  {t("composer.queue.choiceClear")}
                                </SelectMenuItem>
                              </SelectMenu>
                            )
                          : null}
                        {item.attachments && item.attachments.length > 0
                          ? (
                              <ul className="mt-1 flex flex-wrap items-center gap-1" data-queue-attachments="">
                                {item.attachments.map(attachment => (
                                  <li
                                    className="flex max-w-full items-center gap-1 rounded bg-surface-subtle pl-1"
                                    key={attachment.path}
                                  >
                                    <button
                                      className={cn(
                                        "inline-flex min-w-0 items-center gap-1 py-0.5 text-[11px] text-ink transition-colors",
                                        onPreviewAttachment ? "hover:text-accent" : "cursor-default",
                                      )}
                                      data-queue-attachment={attachment.path}
                                      disabled={!onPreviewAttachment}
                                      onClick={() => onPreviewAttachment?.(attachment)}
                                      title={t("composer.queue.previewAttachment", { name: attachment.name })}
                                      type="button"
                                    >
                                      {attachment.kind === "image"
                                        ? <ImageIcon className="size-2.5 shrink-0 text-ink-muted" />
                                        : <Paperclip className="size-2.5 shrink-0 text-ink-muted" />}
                                      <span className="truncate">{attachment.name}</span>
                                    </button>
                                  </li>
                                ))}
                              </ul>
                            )
                          : null}
                        {item.parked && onRelease
                          ? (
                              <button
                                className="mt-0.5 inline-flex items-center gap-1 rounded px-1 py-0.5 text-[11px] font-medium text-warning transition-colors hover:bg-warning-soft"
                                data-queue-release=""
                                onClick={() => onRelease(item.id)}
                                type="button"
                              >
                                <Play className="size-3" />
                                {t("composer.queue.release")}
                              </button>
                            )
                          : null}
                      </div>
                      {/* Nothing to edit in a message that is only files: an
                          empty save is a removal, and the bin beside it says so
                          plainly. The control is left out rather than shown
                          dead, so the row never offers an edit that cannot
                          express what the message is. */}
                      {item.content
                        ? (
                            <button
                              aria-label={t("composer.queue.edit")}
                              className={cn(
                                "inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted transition-colors",
                                "hover:bg-surface-subtle hover:text-ink",
                              )}
                              data-queue-edit=""
                              onClick={() => startEditing(item)}
                              title={t("composer.queue.edit")}
                              type="button"
                            >
                              <Pencil className="size-3" />
                            </button>
                          )
                        : null}
                      <button
                        aria-label={t("composer.queue.remove")}
                        className="inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-danger"
                        data-queue-remove=""
                        onClick={() => onRemove(item.id)}
                        title={t("composer.queue.remove")}
                        type="button"
                      >
                        <Trash2 className="size-3" />
                      </button>
                    </>
                  )}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
