import type { AgentActivityItem, AgentMessage, MessageSegment } from "@future-os/thread-projection";
import { useEffect, useMemo, useRef, useState } from "react";
import { fetchRemoteToolTarget } from "./remoteToolTarget";

/**
 * What each tool call on a host did, filled back into a projected transcript.
 *
 * The shared projector reads a tool call's target from its arguments, and the
 * host's history page is byte-budgeted: a long conversation arrives with a shell
 * call's identity (its call id and run) and no arguments, so those rows would
 * render as "Ran a command" with nothing after it. This fetches the missing ones
 * from the host and puts them where the renderer already looks.
 *
 * It works on the *projected* messages rather than on raw entries because that is
 * what the transcript draws; filling the entries instead would mean doing this
 * again after every projection, on every streaming push.
 */

/** One call whose arguments have to be fetched. */
export interface PendingTarget {
  runId: string;
  toolCallId: string;
}

function collectFromItem(item: AgentActivityItem, out: PendingTarget[]): void {
  // A collapsed row ("Ran 4 commands") carries the calls it stands for; a lean
  // page strips the arguments of each, and the collapsed row's own expansion is
  // where they are read.
  for (const child of item.children ?? [])
    collectFromItem(child, out);
  const already = (item.target ?? "").trim() !== "";
  if (!already && item.toolCallId && item.runId)
    out.push({ runId: item.runId, toolCallId: item.toolCallId });
}

/** Every call in these messages whose target is not in the transcript already. */
export function pendingTargets(messages: AgentMessage[]): PendingTarget[] {
  const out: PendingTarget[] = [];
  const seen = new Set<string>();
  for (const message of messages) {
    for (const segment of message.segments ?? []) {
      if (segment.kind !== "activity")
        continue;
      const before = out.length;
      collectFromItem(segment.item, out);
      // A call can appear in a segment and again in a collapsed group; asking
      // twice would be two requests for one answer.
      for (let index = out.length - 1; index >= before; index -= 1) {
        const key = `${out[index]!.runId}::${out[index]!.toolCallId}`;
        if (seen.has(key))
          out.splice(index, 1);
        else
          seen.add(key);
      }
    }
    // The legacy path: a message with no segments carries its activity list.
    for (const item of message.activityItems ?? [])
      collectFromItem(item, out);
  }
  return out;
}

function fillItem(item: AgentActivityItem, found: ReadonlyMap<string, string>): AgentActivityItem {
  const children = item.children?.map(child => fillItem(child, found));
  const target = item.target ?? (item.toolCallId ? found.get(item.toolCallId) : undefined);
  if (target === item.target && children === item.children)
    return item;
  return { ...item, target, detail: item.detail ?? target, ...(children ? { children } : {}) };
}

/**
 * Why a reply ended badly, put where the transcript renders it.
 *
 * The host reports the reason as `run.error`; this app's message renderer shows
 * `terminationNotice` (set on the local path by `applyJournalRunOutcomes`, which
 * reads *this* machine's run rows and therefore cannot run for a remote
 * session). Without this the reply is marked failed, offers Retry, and says
 * nothing about what went wrong — the reader is told to retry a run whose cause
 * is on the other machine.
 */
function withTermination(message: AgentMessage): AgentMessage {
  const reason = message.runError?.trim();
  if (!reason || message.terminationNotice)
    return message;
  return { ...message, terminationNotice: reason };
}

function fillSegment(segment: MessageSegment, found: ReadonlyMap<string, string>): MessageSegment {
  if (segment.kind !== "activity")
    return segment;
  const item = fillItem(segment.item, found);
  return item === segment.item ? segment : { ...segment, item };
}

/**
 * The same messages, with every target the host has provided filled in.
 *
 * Returns the input array untouched when nothing changed, so a component that
 * depends on the result is not re-rendered once per streaming push.
 */
export function withRemoteTargets(
  messages: AgentMessage[],
  found: ReadonlyMap<string, string>,
): AgentMessage[] {
  let changed = false;
  const next = messages.map((message) => {
    const withReason = withTermination(message);
    if (withReason !== message)
      changed = true;
    if (found.size === 0)
      return withReason;
    const segments = withReason.segments?.map((segment) => {
      const filled = fillSegment(segment, found);
      if (filled !== segment)
        changed = true;
      return filled;
    });
    if (!segments)
      return withReason;
    return { ...withReason, segments };
  });
  return changed ? next : messages;
}

/**
 * Fetch the targets this transcript is missing, once each.
 *
 * The transcript itself is returned unchanged until a fetch lands, so a
 * conversation renders immediately and gains its commands as they arrive.
 */
export function useRemoteActivityTargets(
  desktopId: string | null,
  sessionId: string | null,
  messages: AgentMessage[],
): Map<string, string> {
  const [found, setFound] = useState<Map<string, string>>(() => new Map());
  const pending = useMemo(() => pendingTargets(messages), [messages]);
  // Keyed so a fetch is asked for once per call, whatever the render count.
  const key = useMemo(
    () => pending.map(item => `${item.runId}::${item.toolCallId}`).sort().join(","),
    [pending],
  );
  /**
   * The calls already asked for.
   *
   * A ref rather than a read of `found`: `found` only gains an entry for a target
   * the host actually reported, so a call with no target would be asked for
   * again on every render — and reading state inside the effect would make the
   * effect depend on it, which is a request per streaming push. The current list
   * is read through a ref too, so the effect runs when the *set* of missing
   * calls changes and not once per delta.
   */
  const requestedRef = useRef(new Set<string>());
  const pendingRef = useRef(pending);
  pendingRef.current = pending;

  useEffect(() => {
    if (!desktopId || !sessionId)
      return;
    let cancelled = false;
    for (const item of pendingRef.current) {
      const address = `${item.runId}::${item.toolCallId}`;
      if (requestedRef.current.has(address))
        continue;
      requestedRef.current.add(address);
      void fetchRemoteToolTarget({
        desktopId,
        runId: item.runId,
        sessionId,
        toolCallId: item.toolCallId,
      }).then((target) => {
        // A host that reports no target is not remembered here: the fetch cache
        // has already stopped asking, and the row simply shows its name.
        if (cancelled || target === null)
          return;
        setFound((current) => {
          if (current.get(item.toolCallId) === target)
            return current;
          const next = new Map(current);
          next.set(item.toolCallId, target);
          return next;
        });
      });
    }
    return () => {
      cancelled = true;
    };
  }, [desktopId, sessionId, key]);

  // A different conversation keeps nothing: the calls belong to the old one, and
  // its ids never repeat, so nothing here would be read again.
  useEffect(() => {
    setFound(new Map());
    requestedRef.current = new Set();
  }, [desktopId, sessionId]);

  return found;
}
