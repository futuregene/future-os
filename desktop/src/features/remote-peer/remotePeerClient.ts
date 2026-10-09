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
  kind: "event" | "presence";
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
      if (typeof entry !== "object" || entry === null) return [];
      const row = entry as Record<string, unknown>;
      const sessionId = row.sessionId;
      if (typeof sessionId !== "string" || !sessionId) return [];
      return [{
        sessionId,
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
