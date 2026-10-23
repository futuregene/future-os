/**
 * Fixtures for the two labels that sit below `text-xs`.
 *
 * These exist for one question: when the reader's text size is "Small", do the
 * pending attachment's size in the composer and an approval card's own label
 * stay readable? They are the only places in the app drawn at the 10 and 11
 * design sizes, so they are the only places the size scale's floor is visible —
 * and neither state is reachable from the demo catalogue on its own:
 *
 *   ?approval=1            a run blocked on the reader's approval, docked above
 *                          the composer (its "Shell command" label is the
 *                          11-design-size one)
 *   ?draft=attachments     a composer draft that already holds two pending
 *                          attachments (their byte sizes are the 10-design-size
 *                          ones)
 *
 * Both reproduce state the shipping code reads: the approval goes through the
 * real reducer from a real `approval_request` event, and the draft is the same
 * `draftStorage` record the composer restores after a visit. Nothing under
 * `src/` is replaced or bypassed.
 */
import { applyStreamEvents, type TimelineState } from "@future-os/remote-client/projection";
import type { StreamEvent } from "../../src/remote/types";
import { desktopDraftKey } from "../../src/remote/desktopDraftKey";
import { demoCredentials } from "./data";
import approvalRequestEvents from "./fixtures/approval-request-events.json";
import pendingAttachmentDraft from "./fixtures/draft-pending-attachments.json";

/**
 * The timeline with the demo conversation's approval request docked on it.
 *
 * `?approval=1` asks for it; every other capture gets its timeline back
 * untouched. Running the event through `applyStreamEvents` rather than
 * injecting the item is the point: the card's text comes from the same
 * projection the desktop feeds a phone, so a capture of it is a capture of the
 * shipping label.
 */
export function withApprovalRequest(search: string, timeline: TimelineState): TimelineState {
  if (new URLSearchParams(search).get("approval") !== "1") return timeline;
  return applyStreamEvents(timeline, approvalRequestEvents as unknown as StreamEvent[]);
}

/**
 * Store the pending-attachment draft where the composer reads it, for
 * `?draft=attachments`.
 *
 * Written under `draftStorage`'s own key and version, so the composer restores
 * it through the shipped path (`loadSessionDraft`) — the fixture seeds the
 * stored draft, it does not fake the composer's state. Called before the first
 * render: the screen reads the draft in a mount effect, and a seed that landed
 * after it would simply be missed.
 */
export function seedComposerDraft(search: string): void {
  if (new URLSearchParams(search).get("draft") !== "attachments") return;
  // The slot is `draftStorage`'s (`src/remote/draftStorage.ts`): its key prefix
  // and the draft version are private to that module, so they are repeated here
  // rather than exported for a fixture's sake. If the storage format moves, this
  // fixture stops restoring — which is what the capture failing to show the two
  // attachments will say.
  const key = `futureos.remote.draft.v1:${desktopDraftKey(demoCredentials.expectedDesktopId)}`;
  window.localStorage.setItem(key, JSON.stringify(pendingAttachmentDraft));
}
