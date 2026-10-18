import { createContext, use } from "react";

/**
 * Where the attachments of the transcript below this point actually live.
 *
 * A remote conversation's message metadata carries the *host's* paths — that is
 * what the host's agent read, and it is the honest record of what was sent. On
 * this machine those paths mean nothing: the file is on the other computer, and
 * the only way to look at it is to fetch it (`previewRemoteFile`, the same
 * transfer the files panel uses). The phone has always done that; this app drew
 * the chips with its own local behaviour instead, so a click ran `openPath` on a
 * path that does not exist here and reported the file as "moved or deleted".
 *
 * A context rather than a prop threaded through `MessageBlock`: the transcript
 * is shared (`RemoteConversationView` and `AgentThread` draw the same block), and
 * only the chip at the bottom of that tree cares. This is the one fact the tree
 * needs from the frame around it.
 */
export interface RemoteAttachmentOrigin {
  /** The machine the paths point at. */
  desktopId: string;
  /** The conversation there — the subject of the file read. */
  sessionId: string;
}

const RemoteAttachmentContext = createContext<RemoteAttachmentOrigin | null>(null);

export const RemoteAttachmentProvider = RemoteAttachmentContext.Provider;

/** The machine this transcript's attachment paths point at, when it is not this one. */
export function useRemoteAttachmentOrigin(): RemoteAttachmentOrigin | null {
  return use(RemoteAttachmentContext);
}
