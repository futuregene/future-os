/**
 * Signal that content shared from another app has landed in the chosen
 * conversation's composer draft (new or existing).
 *
 * The content itself travels through `draftStorage` (the composer's existing,
 * tested restore path). This signal exists for one case that path cannot cover:
 * when the app is *already* showing the destination conversation, the composer's
 * draft key does not change, so nothing would re-read the draft. The chat
 * screen's key includes this revision, so that composer remounts and restores.
 */
let revision = 0;
const listeners = new Set<() => void>();

/** Announce that a shared payload is waiting in the composer draft. */
export function markShareLanded(): void {
  revision += 1;
  for (const listener of listeners) listener();
}

/** Bumps whenever a share lands; the chat screen's key includes it. */
export function shareLandedRevision(): number {
  return revision;
}

export function subscribeShareLanded(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
