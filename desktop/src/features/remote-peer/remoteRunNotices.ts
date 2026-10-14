import type { AgentMessage } from "@future-os/thread-projection";

/**
 * The host's own facts about a reply, put where this app's transcript draws them.
 *
 * A remote transcript is drawn by the local renderer over messages the shared
 * projector produced (`entriesToMessages`), so anything the local path fills in
 * from *this* machine's stores is missing for a conversation that lives on
 * another one. What is missing today is the reason a run ended badly: the host
 * reports it as `run.error`, and the local path's own notice is written from
 * local run rows (`applyJournalRunOutcomes`), which cannot run for a remote
 * session. Without this, a reply is marked failed, offers Retry, and says nothing
 * about what went wrong — the reader is told to retry a run whose cause is on the
 * other machine.
 *
 * A tool row's *target* used to be filled in here too, by fetching every call the
 * page did not carry. It is not any more: the host trims a lean page, and the
 * command a row shows is now read when the reader opens that row, which is what
 * keeps a conversation's open from costing one command per call it contains (see
 * `ActivityTargetResolver`).
 */

/**
 * Why a reply ended badly, put where the transcript renders it.
 *
 * The host reports the reason as `run.error`; this app's message renderer shows
 * `terminationNotice` (set on the local path by `applyJournalRunOutcomes`, which
 * reads *this* machine's run rows and therefore cannot run for a remote session).
 */
function withTermination(message: AgentMessage): AgentMessage {
  const reason = message.runError?.trim();
  if (!reason || message.terminationNotice)
    return message;
  return { ...message, terminationNotice: reason };
}

/**
 * The same messages, with every host-reported notice this machine cannot derive
 * filled in.
 *
 * Returns the input array untouched when nothing changed, so a component that
 * depends on the result is not re-rendered once per streaming push.
 */
export function withRemoteRunNotices(messages: AgentMessage[]): AgentMessage[] {
  let changed = false;
  const next = messages.map((message) => {
    const withReason = withTermination(message);
    if (withReason !== message)
      changed = true;
    return withReason;
  });
  return changed ? next : messages;
}
