import type { RemoteCatalog, RemoteSessionRow } from "./mergeConversations";
import { invokeCommand } from "../../integrations/tauri/invoke";

/**
 * The client-role command surface: "who am I connected to", as opposed to
 * `features/remote/remoteClient` which answers "who is connected to me".
 * Keeping them in separate modules is what stops a screen from reading the
 * wrong status — they are one word apart and mean opposite things.
 */

export interface RemotePeer {
  desktopId: string;
  pairId: string;
  /** The user's local name for this host; never sent anywhere. */
  name: string | null;
  /** A glyph from the fixed picker (`peerIcons.ts`), shown in place of the name. */
  icon: string | null;
  connected: boolean;
  /** Why the last connection attempt failed, if it did. */
  error: string | null;
  bridgeInstanceId: string | null;
  /** Capabilities the host declared. Optional actions are gated on these. */
  features: string[];
  /** The host's own report that its agent is reachable (`LC003` if not). */
  agentAvailable: boolean;
}

/** One decrypted push from a host, as the backend forwards it. */
export interface RemotePeerEvent {
  desktopId: string;
  /**
   * `event` for a session event, `presence` for a liveness tick, `resumed` for a
   * link that came back after a drop.
   *
   * `resumed` is separate because neither of the others can express it: a
   * presence tick arrives on a healthy link as well, and a session event says
   * nothing about the link. It is the only signal that what the host sent while
   * the link was down is *missing* rather than merely absent.
   */
  kind: "event" | "presence" | "resumed";
  payload: unknown;
}

export function listRemotePeers(): Promise<RemotePeer[]> {
  return invokeCommand<RemotePeer[]>("remote_peer_list");
}

export function pairRemotePeer(invitation: string): Promise<RemotePeer> {
  return invokeCommand<RemotePeer>("remote_peer_pair", { invitation });
}

export function connectRemotePeer(desktopId: string): Promise<RemotePeer> {
  return invokeCommand<RemotePeer>("remote_peer_connect", { desktopId });
}

export function disconnectRemotePeer(desktopId: string): Promise<void> {
  return invokeCommand<void>("remote_peer_disconnect", { desktopId });
}

/**
 * Resolves to a warning string when the local pairing is gone but the
 * platform-side revoke could not be delivered — the caller should say so
 * rather than claim a clean removal.
 */
export function unpairRemotePeer(desktopId: string): Promise<string | null> {
  return invokeCommand<string | null>("remote_peer_unpair", { desktopId });
}

/**
 * `name`/`icon` are optional-patch semantics: omit to leave alone, pass `""` to
 * clear. That is why they are spread conditionally rather than passed as
 * `undefined`.
 */
export function setRemotePeerLabel(
  desktopId: string,
  patch: { name?: string; icon?: string },
): Promise<void> {
  return invokeCommand<void>("remote_peer_set_label", {
    desktopId,
    ...(patch.name === undefined ? {} : { name: patch.name }),
    ...(patch.icon === undefined ? {} : { icon: patch.icon }),
  });
}

interface RawCatalog {
  desktopId?: string;
  pairId?: string;
  sessions?: unknown;
}

/**
 * A host's session snapshot, normalised to the shape the merge expects.
 *
 * The rows come from the host as JSON with optional fields; this is the single
 * place that decides what a missing field means, so the merge (and the UI) can
 * assume a total shape. In particular a session without a `sessionId` is
 * dropped rather than becoming a row that cannot be opened.
 */
export async function fetchRemoteSessions(desktopId: string): Promise<RemoteCatalog> {
  const raw = await invokeCommand<RawCatalog>("remote_peer_sessions", { desktopId });
  const sessions = Array.isArray(raw?.sessions) ? raw.sessions : [];
  return {
    desktopId,
    sessions: sessions.flatMap((entry): RemoteSessionRow[] => {
      if (typeof entry !== "object" || entry === null)
        return [];
      const row = entry as Record<string, unknown>;
      const sessionId = row.sessionId;
      if (typeof sessionId !== "string" || !sessionId)
        return [];
      return [{
        sessionId,
        ...(typeof row.threadId === "string" && row.threadId ? { threadId: row.threadId } : {}),
        title: typeof row.title === "string" && row.title ? row.title : sessionId,
        mode: row.mode === "workspace" ? "workspace" : "chat",
        ...(typeof row.workspaceId === "string" ? { workspaceId: row.workspaceId } : {}),
        pinned: row.pinned === true,
        streaming: row.streaming === true,
        ...(typeof row.status === "string" ? { status: row.status } : {}),
        lastMessageAt: typeof row.lastMessageAt === "number" ? row.lastMessageAt : null,
      }];
    }),
  };
}

/**
 * How a remote conversation is addressed: the host's **session** id, plus its
 * **thread** id where the host has one.
 *
 * These are two different strings on the host, and they are not
 * interchangeable: prompt, abort and history reads are session-scoped, while
 * pin and delete go through the host's thread store. `threadId` is `null` only
 * on a host that predates the catalogue's thread id, and pin/delete are refused
 * rather than falling back to the session id — sending the session id where a
 * thread id is expected is how the wrong record gets pinned.
 */
export interface RemoteSessionAddress {
  sessionId: string;
  threadId: string | null;
}

/**
 * The host routes that act on one of its conversations.
 *
 * All of them are requests on the addressed session's lane, so a host that
 * refuses one rejects the promise and the caller surfaces it — none of these is
 * fire-and-forget.
 */
export function renameRemoteConversation(
  desktopId: string,
  address: RemoteSessionAddress,
  name: string,
): Promise<unknown> {
  return requestRemotePeer(
    desktopId,
    { type: "set_session_name", sessionId: address.sessionId, name },
    address.sessionId,
  );
}

/** Refuses when the host reported no thread id: see `RemoteSessionAddress`. */
export async function pinRemoteConversation(
  desktopId: string,
  address: RemoteSessionAddress,
  pinned: boolean,
): Promise<unknown> {
  const threadId = requireThreadId(address);
  return requestRemotePeer(
    desktopId,
    { type: "set_session_pinned", threadId, pinned },
    address.sessionId,
  );
}

/** Idempotent on the host: deleting an already-deleted thread succeeds. */
export async function deleteRemoteConversation(
  desktopId: string,
  address: RemoteSessionAddress,
): Promise<unknown> {
  const threadId = requireThreadId(address);
  return requestRemotePeer(desktopId, { type: "delete_session", threadId }, address.sessionId);
}

/**
 * Both callers above are `async` so the refusal arrives as a *rejection*. A
 * half-synchronous API — where one argument makes the call throw and another
 * makes it reject — is caught differently by `try/catch` and by `.catch()`, and
 * the bug that produces is a pin that silently never happened.
 */
function requireThreadId(address: RemoteSessionAddress): string {
  if (!address.threadId)
    throw new Error("remote_conversation_without_thread_id");
  return address.threadId;
}

/** Stop whatever run is in flight for a conversation on that host. */
export function abortRemoteRun(desktopId: string, sessionId: string): Promise<unknown> {
  return requestRemotePeer(desktopId, { type: "abort", sessionId }, sessionId);
}

/**
 * The two ids a host chooses for a conversation it created for us.
 *
 * Both prompt-with-no-session and fork answer with these: the client cannot mint
 * a host-side session id, so the ack is the only way to learn the conversation.
 * `threadId` is empty only on a host that predates the field.
 */
export interface RemoteConversationIds {
  sessionId: string;
  threadId: string;
}

/**
 * Send a prompt to a conversation, or start one when `sessionId` is empty —
 * which is the host's own signal to create the thread.
 */
export async function promptRemoteConversation(
  desktopId: string,
  sessionId: string,
  message: string,
): Promise<RemoteConversationIds> {
  const ack = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "prompt", sessionId, message },
    sessionId || "new",
  );
  const created = ack?.sessionId;
  if (typeof created !== "string" || !created)
    throw new Error("remote_prompt_without_session_id");
  return { sessionId: created, threadId: typeof ack.threadId === "string" ? ack.threadId : "" };
}

/**
 * Fork a conversation at a settled turn, on the host that owns it.
 *
 * `sourceEntryId` must be the host's **persisted** user entry id (see
 * `persistedEntryIds` on the timeline): the host resolves it against its own
 * store, so an event id from a live push is refused. The host answers with the
 * child's ids, which is the only way to learn them — the client cannot mint a
 * host-side session id.
 */
export async function forkRemoteConversation(
  desktopId: string,
  sessionId: string,
  sourceEntryId: string,
): Promise<RemoteConversationIds> {
  const ack = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "fork_session", sessionId, sourceEntryId },
    sessionId,
  );
  const created = ack?.sessionId;
  if (typeof created !== "string" || !created)
    throw new Error("remote_fork_without_session_id");
  return { sessionId: created, threadId: typeof ack.threadId === "string" ? ack.threadId : "" };
}

/**
 * Ask the host to compact a conversation's context.
 *
 * The answer is an *acknowledgement*, not a result: the host accepts the
 * request and then commits, fails, or finds nothing to do, and reports which
 * only through its own events. So a promise that resolves means "the host took
 * it", and the caller waits on the timeline's `compacting` rather than treating
 * this as done.
 *
 * An ack that carries no `operationId` is not a real acceptance — the host uses
 * it to correlate the events that follow — and is rejected rather than reported
 * as success.
 */
export async function compactRemoteConversation(
  desktopId: string,
  sessionId: string,
): Promise<void> {
  const ack = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "compact_context", sessionId },
    sessionId,
  );
  const accepted = ack?.accepted === true
    && typeof ack.operationId === "string"
    && ack.operationId.length > 0;
  if (!accepted)
    throw new Error("remote_compaction_not_accepted");
}

/**
 * One entry in a session directory on a host.
 *
 * `path` is the host's own absolute path, and it is what a download request
 * addresses — the client never joins paths itself, because the host's separator
 * and case rules are the host's.
 */
export interface RemoteFileEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
}

export interface RemoteFileListing {
  /** The session root on the host, for showing where the user is. */
  rootPath: string;
  /** The directory listed, absolute on the host. */
  path: string;
  entries: RemoteFileEntry[];
}

/**
 * List one directory inside a session on a host.
 *
 * An absent `path` means the session's own root; the backend turns that into the
 * empty path the host reads as "the session directory".
 */
export async function listRemoteSessionFiles(
  desktopId: string,
  sessionId: string,
  path?: string,
): Promise<RemoteFileListing> {
  const raw = await invokeCommand<Record<string, unknown>>("remote_peer_list_files", {
    desktopId,
    sessionId,
    ...(path === undefined ? {} : { path }),
  });
  const entries = Array.isArray(raw?.entries) ? raw.entries : [];
  return {
    rootPath: typeof raw?.rootPath === "string" ? raw.rootPath : "",
    path: typeof raw?.path === "string" ? raw.path : "",
    // A row without a name or path could not be opened or downloaded, so it is
    // dropped rather than rendered as a dead entry.
    entries: entries.flatMap((entry): RemoteFileEntry[] => {
      if (typeof entry !== "object" || entry === null)
        return [];
      const row = entry as Record<string, unknown>;
      if (typeof row.name !== "string" || !row.name)
        return [];
      if (typeof row.path !== "string" || !row.path)
        return [];
      return [{
        name: row.name,
        path: row.path,
        isDir: row.isDir === true,
        size: typeof row.size === "number" ? row.size : 0,
      }];
    }),
  };
}

/**
 * Pull a file from a host and write it to a local path.
 *
 * Resolves to the name the *host* gave the file, which can differ from the one
 * asked for (a preview variant is renamed on the host side). Reporting the name
 * the user chose instead would leave them looking for a file that is not there.
 */
export function downloadRemoteFile(input: {
  desktopId: string;
  sessionId: string;
  path: string;
  name: string;
  destination: string;
  variant?: string;
}): Promise<string> {
  return invokeCommand<string>("remote_peer_download_file", {
    desktopId: input.desktopId,
    sessionId: input.sessionId,
    path: input.path,
    name: input.name,
    destination: input.destination,
    ...(input.variant === undefined ? {} : { variant: input.variant }),
  });
}

/**
 * Run one command on one host. The lane is the session the command addresses
 * (`list` for catalogue reads) — the backend builds the subject, so no subject
 * string is ever assembled in the frontend.
 */
export function requestRemotePeer<T = unknown>(
  desktopId: string,
  command: Record<string, unknown>,
  lane: string,
): Promise<T> {
  return invokeCommand<T>("remote_peer_request", { desktopId, command, lane });
}
