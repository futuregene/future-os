import { Check, Pencil, Play, RotateCcw, Trash2 } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

/** One row of the panel: a message waiting for the run to finish. */
export interface SendQueuePanelItem {
  id: string;
  /** The text as it will be sent (empty for an attachment-only message). */
  content: string;
  /**
   * How many files travel with it — `undefined` when the queue that holds the
   * row does not report it.
   *
   * A row read back from the Agent's own queue has no count to show: `get_state`
   * answers with a message's text and its acceptance time, never the uploads it
   * was sent with. Zero used to be hard-coded for those rows, which claimed a
   * message carried no files when it may well have carried several, so the
   * absence is now spelled as absence.
   */
  attachmentCount?: number;
  /**
   * Parked by the Agent: it waited too long to run by itself and now starts only
   * when the reader says so. Undefined means "will run at the next turn
   * boundary", which is what every row is on a host without that rule.
   */
  parked?: boolean;
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
  onRelease,
  onRemove,
  onRetry,
  onUpdate,
}: {
  /**
   * The queue behind these rows does not say which files a message carries, so
   * an edit — a removal and a re-send, the only kind there is — sends the new
   * text and leaves those files behind. Said in the editor, where the reader is
   * about to make that trade, rather than as a number the panel cannot know.
   */
  attachmentsUnreported?: boolean;
  /** The last automatic delivery failed; offer a retry instead of the hint. */
  failed?: boolean;
  items: readonly SendQueuePanelItem[];
  /** Start a parked row now. Offered only when the queue can hold one (Agent-side queues). */
  onRelease?: (id: string) => void;
  onRemove: (id: string) => void;
  onRetry?: () => void;
  onUpdate: (id: string, content: string) => void;
}) {
  const { t } = useTranslation("agent");
  // Only one row is edited at a time — the panel is a list of what is about to
  // leave, not a second editor.
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingText, setEditingText] = useState("");

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
                    so the merge this line describes would never happen to it. */}
                {items.some(item => item.parked)
                  ? t("composer.queue.hintParked")
                  : t("composer.queue.hint")}
              </span>
            )}
      </header>
      <ol className="mt-1.5 space-y-1">
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
                      {attachmentsUnreported
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
                        {item.attachmentCount !== undefined && item.attachmentCount > 0
                          ? (
                              <p className="text-[11px] leading-4 text-ink-muted">
                                {t("composer.queue.attachments", { count: item.attachmentCount })}
                              </p>
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
