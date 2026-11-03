import { useCallback, useSyncExternalStore } from "react";

/**
 * The send queue: messages held for a conversation whose run is still in flight.
 *
 * A run answers one prompt at a time, so a send while one streams has nowhere to
 * go. The composer's answer is a queue rather than a refusal: the message is
 * held here, and the conversation sends the queue as **one** message the moment
 * the run settles — or straight away when the reader interrupts the run.
 *
 * Held per conversation, and deliberately in memory only: an attachment is a
 * path or an upload id that stops meaning anything across a restart, so a queue
 * that survived one could send a message whose files are gone. Switching
 * conversations keeps the entry (the run keeps going in the background); the
 * conversation's own view flushes it when it is next idle.
 *
 * The store is keyed rather than React state because the two places that write
 * it — the composer that queues, and the view that flushes when the run ends —
 * are different subtrees, and the panel that edits it is a third.
 */

/** One message waiting for its conversation's run to finish. */
export interface QueuedSend<TAttachment> {
  readonly id: string;
  /** The message as it will be sent, already expanded by the writer. */
  readonly content: string;
  readonly attachments: readonly TAttachment[];
  readonly createdAt: number;
}

/** What a conversation's queue currently holds. */
export interface SendQueueState<TAttachment> {
  readonly items: readonly QueuedSend<TAttachment>[];
  /**
   * The last automatic delivery failed and the items stayed put. Auto-flush
   * waits until the reader retries (or edits/removes something), so a queue the
   * host keeps refusing cannot turn into a retry loop.
   */
  readonly failed: boolean;
}

export interface SendQueuePayload<TAttachment> {
  content: string;
  attachments?: readonly TAttachment[];
}

/** The most messages one conversation may hold. */
const MAX_QUEUED = 20;

export interface SendQueueStore<TAttachment> {
  /** Subscribe to one conversation's queue changes. */
  subscribe: (key: string, listener: () => void) => () => void;
  /** The current state (identity-stable until something changes). */
  snapshot: (key: string | null) => SendQueueState<TAttachment>;
  /** Append a message; null when it is empty or the queue is full. */
  enqueue: (key: string, payload: SendQueuePayload<TAttachment>) => QueuedSend<TAttachment> | null;
  /** Replace a queued message's text (empty text with no attachments removes it). */
  update: (key: string, id: string, content: string) => void;
  /** Drop one queued message. */
  remove: (key: string, id: string) => void;
  /**
   * Take everything out of the queue for delivery. The caller owns the returned
   * items and must call `restore` if delivery fails.
   */
  take: (key: string) => readonly QueuedSend<TAttachment>[];
  /** Put taken items back at the front and stop automatic retries. */
  restore: (key: string, items: readonly QueuedSend<TAttachment>[]) => void;
  /** Retry permission for a failed automatic delivery. */
  clearFailure: (key: string) => void;
  /** Empty the queue (e.g. the conversation was deleted). */
  clear: (key: string) => void;
}

export function createSendQueueStore<TAttachment>(): SendQueueStore<TAttachment> {
  const states = new Map<string, SendQueueState<TAttachment>>();
  const listeners = new Map<string, Set<() => void>>();
  const empty: SendQueueState<TAttachment> = { failed: false, items: [] };
  let sequence = 0;

  function read(key: string): SendQueueState<TAttachment> {
    return states.get(key) ?? empty;
  }

  function publish(key: string, next: SendQueueState<TAttachment>): void {
    states.set(key, next);
    const set = listeners.get(key);
    if (!set)
      return;
    for (const listener of [...set])
      listener();
  }

  function enqueue(key: string, payload: SendQueuePayload<TAttachment>): QueuedSend<TAttachment> | null {
    const content = payload.content.trim();
    const attachments = payload.attachments ?? [];
    if (!content && attachments.length === 0)
      return null;
    const current = read(key);
    if (current.items.length >= MAX_QUEUED)
      return null;
    sequence += 1;
    const item: QueuedSend<TAttachment> = {
      id: `queued-${Date.now().toString(36)}-${sequence}`,
      // The text is trimmed once, here: every later reader (the panel, the
      // joined message) then agrees about what was written.
      content,
      attachments: [...attachments],
      createdAt: Date.now(),
    };
    publish(key, { failed: false, items: [...current.items, item] });
    return item;
  }

  function update(key: string, id: string, content: string): void {
    const current = read(key);
    const item = current.items.find(entry => entry.id === id);
    if (!item)
      return;
    const trimmed = content.trim();
    if (!trimmed && item.attachments.length === 0) {
      publish(key, { failed: false, items: current.items.filter(entry => entry.id !== id) });
      return;
    }
    publish(key, {
      failed: false,
      items: current.items.map(entry => entry.id === id ? { ...entry, content: trimmed } : entry),
    });
  }

  function remove(key: string, id: string): void {
    const current = read(key);
    if (!current.items.some(entry => entry.id === id))
      return;
    publish(key, { failed: false, items: current.items.filter(entry => entry.id !== id) });
  }

  function take(key: string): readonly QueuedSend<TAttachment>[] {
    const current = read(key);
    if (current.items.length === 0)
      return [];
    publish(key, empty);
    return current.items;
  }

  function restore(key: string, items: readonly QueuedSend<TAttachment>[]): void {
    if (items.length === 0)
      return;
    publish(key, { failed: true, items: [...items, ...read(key).items] });
  }

  function clearFailure(key: string): void {
    const current = read(key);
    if (!current.failed)
      return;
    publish(key, { failed: false, items: current.items });
  }

  function clear(key: string): void {
    publish(key, empty);
  }

  return {
    clear,
    clearFailure,
    enqueue,
    remove,
    restore,
    snapshot: key => key ? read(key) : empty,
    subscribe: (key, listener) => {
      let set = listeners.get(key);
      if (!set) {
        set = new Set();
        listeners.set(key, set);
      }
      set.add(listener);
      return () => {
        set.delete(listener);
        if (set.size === 0)
          listeners.delete(key);
      };
    },
    take,
    update,
  };
}

/**
 * The queued messages as one message: each on its own paragraph, in queue order.
 *
 * Blank entries are dropped rather than becoming empty paragraphs — an
 * attachment-only entry has no words to contribute, and its files travel
 * separately.
 */
export function joinQueuedContent(items: readonly { content: string }[]): string {
  return items.map(item => item.content.trim()).filter(Boolean).join("\n\n");
}

/** The attachments of every queued message, in queue order. */
export function collectQueuedAttachments<TAttachment>(items: readonly QueuedSend<TAttachment>[]): TAttachment[] {
  return items.flatMap(item => [...item.attachments]);
}

/** Subscribe a component to one conversation's queue. */
export function useSendQueueState<TAttachment>(
  store: SendQueueStore<TAttachment>,
  key: string | null,
): SendQueueState<TAttachment> {
  const subscribe = useCallback(
    (listener: () => void) => key ? store.subscribe(key, listener) : () => {},
    [key, store],
  );
  const getSnapshot = useCallback(() => store.snapshot(key), [key, store]);
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}
