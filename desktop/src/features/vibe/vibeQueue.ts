import type { MessageAttachment } from "@future-os/thread-projection";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { loadComposerDraft, saveComposerDraft } from "../agent/composerDraft";

/**
 * The panel's send queue: one prompt per conversation, waiting for the run in flight
 * to finish — and then **handing the sentence back**, never sending it.
 *
 * Why "hand it back" rather than "send it": a message dictated from a sofa is exactly
 * the kind of thing that must not arrive without a last look. So when the run ends,
 * the queued text goes back into that conversation's input box and the reader presses
 * `⏎` if they still mean it. There is no window in which a sentence they had second
 * thoughts about is already gone, and no timer to tune.
 *
 * The five properties this module is built around:
 *
 * - **Visible**: `queued` and `returned` are state the panel draws on the cell, so a
 *   queued sentence is never "said but invisible".
 * - **Cancellable, and cancelling is not discarding**: cancelling hands the text back
 *   to the box the same way a finished run does. When the box already holds something
 *   new, cancelling is refused rather than allowed to overwrite it — see
 *   `CancelResult`.
 * - **One per conversation**: `enqueue` refuses a second one instead of replacing it,
 *   because replacing is how a sentence gets swallowed.
 * - **The last moment is the reader's**: nothing here sends anything, ever.
 * - **Failure keeps it**: a run that failed or was interrupted reaches the same "not
 *   running" check, so the text comes back rather than being dropped or re-sent.
 */

export interface QueuedPrompt {
  text: string;
  attachments: MessageAttachment[];
}

export type QueuedMap = ReadonlyMap<string, QueuedPrompt>;

/** What `enqueue` did. */
export type QueueResult = "queued" | "refused";

/** What `cancel` did. */
export type CancelResult = "cancelled" | "none" | "busy";

/**
 * Add one prompt for `threadId`, unless that conversation already has one.
 *
 * Pure, and the refusal is the point: a second `⏎` while one waits must not overwrite
 * it, and the caller surfaces "there is already one queued" instead of pretending.
 */
export function enqueuePrompt(queue: QueuedMap, threadId: string, prompt: QueuedPrompt): { queue: Map<string, QueuedPrompt>; result: QueueResult } {
  if (queue.has(threadId))
    return { queue: new Map(queue), result: "refused" };
  const next = new Map(queue);
  next.set(threadId, prompt);
  return { queue: next, result: "queued" };
}

/** Remove and return one conversation's queued prompt. */
export function takePrompt(queue: QueuedMap, threadId: string): { prompt: QueuedPrompt | null; queue: Map<string, QueuedPrompt> } {
  const prompt = queue.get(threadId) ?? null;
  if (!prompt)
    return { prompt: null, queue: new Map(queue) };
  const next = new Map(queue);
  next.delete(threadId);
  return { prompt, queue: next };
}

/** Whether a run status means "this conversation is busy right now". */
export function isRunningStatus(status: string | undefined): boolean {
  return status === "queued" || status === "running" || status === "waiting_approval";
}

/** Whether a conversation's own input box is empty (so a held sentence fits). */
export function draftIsEmpty(threadId: string): boolean {
  return (loadComposerDraft(threadId)?.text ?? "").trim().length === 0;
}

/**
 * The conversations whose queued text should go back into their input box now.
 *
 * "Now" is: the conversation is not running, and its box is empty. The second half
 * matters — a reader may have started typing something new while the run finished, and
 * putting the queued sentence on top of it would be a second message they never agreed
 * to. When the box is busy the entry waits; the next check (the next run's end) picks
 * it up.
 */
export function dueReturns(
  queue: QueuedMap,
  isRunning: (threadId: string) => boolean,
  draftTextOf: (threadId: string) => string,
): string[] {
  const due: string[] = [];
  for (const threadId of queue.keys()) {
    if (isRunning(threadId))
      continue;
    if (draftTextOf(threadId).trim().length > 0)
      continue;
    due.push(threadId);
  }
  return due;
}

/** Put one prompt into a conversation's draft (where an unsent message lives). */
export function returnToDraft(threadId: string, prompt: QueuedPrompt): void {
  saveComposerDraft(threadId, { attachments: prompt.attachments, text: prompt.text });
}

/**
 * Take one conversation's held sentence out of the queue and back to its box.
 *
 * A refusal (`busy`) is not a failure: the box already holds something the reader typed
 * since the sentence was queued, and putting the held text on top of it would replace a
 * message they are writing with one they had second thoughts about. The entry stays —
 * visible, cancellable later — and nothing is lost either way.
 */
export function cancelPrompt(
  queue: QueuedMap,
  threadId: string,
  draftTextOf: (threadId: string) => string,
): { prompt: QueuedPrompt | null; queue: Map<string, QueuedPrompt>; result: CancelResult } {
  const prompt = queue.get(threadId) ?? null;
  if (!prompt)
    return { prompt: null, queue: new Map(queue), result: "none" };
  if (draftTextOf(threadId).trim().length > 0)
    return { prompt, queue: new Map(queue), result: "busy" };
  return { prompt, queue: takePrompt(queue, threadId).queue, result: "cancelled" };
}

export interface VibeQueue {
  queued: QueuedMap;
  /** Conversations whose queued sentence has just gone back into the input box. */
  returned: ReadonlySet<string>;
  /** Queue a prompt; `refused` when this conversation already has one. */
  enqueue: (threadId: string, prompt: QueuedPrompt) => QueueResult;
  /** Hand one back to its input box now — cancelling never discards the text. */
  cancel: (threadId: string) => CancelResult;
  /** The reader has seen the returned text; stop marking the cell. */
  acknowledge: (threadId: string) => void;
}

/**
 * The queue, wired to the conversations' run statuses.
 *
 * `statusSignature` is a value that changes whenever any conversation's run status does
 * (the panel passes the statuses joined). It exists so the hand-back check can be an
 * ordinary effect with an honest dependency list: the question is "has anything
 * stopped running", and neither the queue nor this hook's own state changes when a run
 * ends somewhere else.
 */
export function useVibeQueue(
  runStatus: (threadId: string) => string | undefined,
  statusSignature: string,
): VibeQueue {
  const [queued, setQueued] = useState<Map<string, QueuedPrompt>>(() => new Map());
  const [returned, setReturned] = useState<ReadonlySet<string>>(() => new Set());
  /**
   * The live queue, for the two callers that must answer **synchronously**.
   *
   * `enqueue` decides whether the composer may clear its box, and `cancel` decides
   * whether the text could be handed back — both are answers to a key press, and a
   * state updater runs later than that.
   */
  const queueRef = useRef<Map<string, QueuedPrompt>>(queued);
  const statusRef = useRef(runStatus);
  useEffect(() => {
    statusRef.current = runStatus;
  }, [runStatus]);

  const commit = useCallback((next: Map<string, QueuedPrompt>) => {
    queueRef.current = next;
    setQueued(next);
  }, []);

  const clearReturned = useCallback((threadId: string) => {
    setReturned((current) => {
      if (!current.has(threadId))
        return current;
      const next = new Set(current);
      next.delete(threadId);
      return next;
    });
  }, []);

  const enqueue = useCallback((threadId: string, prompt: QueuedPrompt): QueueResult => {
    const { queue, result } = enqueuePrompt(queueRef.current, threadId, prompt);
    if (result === "queued") {
      commit(queue);
      // A queued sentence supersedes the "handed back" mark: the cell now says what is
      // actually true.
      clearReturned(threadId);
    }
    return result;
  }, [clearReturned, commit]);

  const cancel = useCallback((threadId: string): CancelResult => {
    const outcome = cancelPrompt(queueRef.current, threadId, threadId => loadComposerDraft(threadId)?.text ?? "");
    if (outcome.result !== "cancelled" || !outcome.prompt)
      return outcome.result;
    commit(outcome.queue);
    returnToDraft(threadId, outcome.prompt);
    return "cancelled";
  }, [commit]);

  const acknowledge = useCallback((threadId: string) => clearReturned(threadId), [clearReturned]);

  // Hand each finished conversation's sentence back, whenever a run ends or something
  // new is queued. It cannot loop: an entry leaves the queue in the same step that
  // returns it, so the next pass finds nothing due.
  useEffect(() => {
    if (queueRef.current.size === 0)
      return;
    const due = dueReturns(
      queueRef.current,
      threadId => isRunningStatus(statusRef.current(threadId)),
      threadId => loadComposerDraft(threadId)?.text ?? "",
    );
    if (due.length === 0)
      return;
    let next = queueRef.current;
    const handed: string[] = [];
    for (const threadId of due) {
      const outcome = takePrompt(next, threadId);
      next = outcome.queue;
      if (outcome.prompt) {
        returnToDraft(threadId, outcome.prompt);
        handed.push(threadId);
      }
    }
    commit(next);
    if (handed.length > 0)
      setReturned(current => new Set([...current, ...handed]));
  }, [commit, queued, statusSignature]);

  return useMemo(
    () => ({ acknowledge, cancel, enqueue, queued, returned }),
    [acknowledge, cancel, enqueue, queued, returned],
  );
}
