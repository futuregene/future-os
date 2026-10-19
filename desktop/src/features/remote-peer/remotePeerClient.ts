import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ProvidersView } from "../../integrations/agent/providers";
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
  /**
   * Whether the *user* turned this host off (its own Disconnect control), as
   * opposed to it being unreachable on its own.
   *
   * The app's liveness pass reads this to decide who to bring back: a host that
   * dropped is retried, and one the user stopped is left off until something
   * asks for it by name. Without the distinction, an automatic pass would keep
   * undoing a deliberate "Disconnect".
   */
  stopped: boolean;
  /** Why the last connection attempt failed, if it did. */
  error: string | null;
  bridgeInstanceId: string | null;
  /** Capabilities the host declared. Optional actions are gated on these. */
  features: string[];
  /** The host's own report that its agent is reachable (`LC003` if not). */
  agentAvailable: boolean;
  /**
   * Whether this device may drive the host, as the host last reported it.
   *
   * Absent on a host that predates the field — the client then assumes what it
   * always did: the machine is its own. Present as `standby` when another paired
   * device holds it, which is a different fact from `connected`: the link is
   * healthy and taking it back needs no reconnection, but nothing on that host
   * can be read or prompted until this device takes over.
   */
  control?: RemotePeerControl;
  /** Who holds the host while this device stands by. */
  holder?: RemotePeerHolder | null;
}

/** `active`: this desktop drives the host. `standby`: another paired device does. */
export type RemotePeerControl = "active" | "standby";

/**
 * The controlling device's self-declared identity, as the host repeats it.
 *
 * Every field is optional — a controller that never declared a name leaves the
 * pair id — and the wording falls back to "another device" rather than
 * inventing one.
 */
export interface RemotePeerHolder {
  pairId?: string;
  name?: string;
  kind?: string;
}

/**
 * One decrypted push from a host, as the backend forwards it.
 */
export interface RemotePeerEvent {
  desktopId: string;
  /**
   * `event` for a session event, `presence` for a liveness tick, `control` for
   * a packet that says the right to drive the host moved, `resumed` for a link
   * that came back after a drop, `catalog` for a sessions or workspaces
   * snapshot.
   *
   * `control` is separate from `presence` because it answers a different
   * question: a tick says the host is alive and mentions, in passing, who may
   * drive it, while this packet exists to say that *that* moved. Waiting for the
   * next tick leaves the machine drawn as usable for up to a second after it
   * stopped being so — long enough for the list to look current when it is not.
   *
   * `resumed` is separate because neither of the others can express it: a
   * presence tick arrives on a healthy link as well, and a session event says
   * nothing about the link. It is the only signal that what the host sent while
   * the link was down is *missing* rather than merely absent.
   */
  kind: "event" | "presence" | "control" | "resumed" | "catalog";
  payload: unknown;
}

/**
 * One event of a run, as the host's own body spells it.
 *
 * The wire shape of the push lane: a host publishes its agent's events as they
 * happen, keyed by the run they belong to and numbered by `idx` within it. A body
 * is trimmed for a client that declared the trimmed lane (`lean_events_v1`): it
 * keeps exactly these keys plus `sessionId`, so a field added here is a field the
 * host must keep sending.
 */
export interface RemoteRunEvent {
  /** The event's JSON payload, exactly as the host sent it (a JSON string). */
  data: string;
  /** Run-scoped event index; the projector orders by this. */
  idx: number;
  /** How many source events this one stands for, when the host merged them. */
  coalescedCount?: number;
  /** The host's event id, which is this event's identity. */
  eventId: string;
  /** The host's run id — what the reply bubble is addressed by. */
  runId: string;
  /** ISO timestamp from the host. */
  timestamp: string;
  /** The host's event type (`text_chunk`, `tool_end`, …). */
  type: string;
}

/**
 * This machine's paired hosts.
 *
 * Validated rather than trusted: a command a backend does not implement answers
 * `null` (the harness mock logs `UNHANDLED COMMAND` and returns null), and a
 * caller that puts that into state feeds `null` to every screen that renders the
 * list — which throws during render and takes the whole application down, not
 * just the remote screen. An empty list is the honest reading of "no answer".
 */
export async function listRemotePeers(): Promise<RemotePeer[]> {
  const raw = await invokeCommand<RemotePeer[] | null>("remote_peer_list");
  if (!Array.isArray(raw))
    return [];
  return raw.map(normalizePeer);
}

/**
 * One peer as the rest of the app may assume it.
 *
 * Applied to the *lists* — `list` and `connect_all` — because those are what the
 * app builds its state from (the merged conversations, the dot, the picker).
 * The single-host answers (`pair`, `connect`) are passed through as they come:
 * every caller of them re-reads the list rather than using the row it got back.
 *
 * Two fields whose *absence* carries a meaning the caller acts on are spelled
 * out here rather than left to a truthiness test at each site: `stopped` (a
 * backend from before the mark existed means "not stopped") and `control` (a
 * host that predates it says nothing, and every client assumed the machine was
 * its own). A word this client does not know is dropped rather than kept: an
 * unknown state is not one to act on.
 */
function normalizePeer(peer: RemotePeer): RemotePeer {
  const normalized: RemotePeer = { ...peer, stopped: peer.stopped === true };
  if (normalized.control !== "active" && normalized.control !== "standby")
    delete normalized.control;
  if (typeof normalized.holder !== "object" || normalized.holder === null)
    delete normalized.holder;
  return normalized;
}

/**
 * Bring every paired host up, and answer with the list as it stands.
 *
 * Two callers, both meaning "everything should be live": the launch pass, and
 * the user's own "reconnect all". It does not fail as a whole — a host this
 * machine cannot reach is reported on its own row (its `error`), which is what
 * keeps one sleeping machine from leaving the others unconnected.
 */
export async function connectAllRemotePeers(): Promise<RemotePeer[]> {
  const raw = await invokeCommand<RemotePeer[] | null>("remote_peer_connect_all");
  return (Array.isArray(raw) ? raw : []).map(normalizePeer);
}

export async function pairRemotePeer(invitation: string): Promise<RemotePeer> {
  return invokeCommand<RemotePeer>("remote_peer_pair", { invitation });
}

export function connectRemotePeer(desktopId: string): Promise<RemotePeer> {
  return invokeCommand<RemotePeer>("remote_peer_connect", { desktopId });
}

export function disconnectRemotePeer(desktopId: string): Promise<void> {
  return invokeCommand<void>("remote_peer_disconnect", { desktopId });
}

/**
 * Ask `desktopId` to let this desktop drive it.
 *
 * The client-role twin of `takeoverRemote` (the host page's "let this device
 * control"): the same `takeover` command a phone sends, from this side of the
 * pairing. It is the only way control comes back without a reconnection, which
 * is why it belongs where the machine's state is explained rather than hidden
 * in a menu.
 *
 * Sent on the `list` lane because it is a pairing-level command — it names no
 * conversation. A refusal (the host went away, or another device moved first) is
 * an ordinary rejection; the caller re-reads the machine rather than assuming
 * the pointer moved.
 */
export async function takeoverRemotePeer(desktopId: string): Promise<void> {
  await requestRemotePeer(desktopId, { type: "takeover" }, "list");
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

/**
 * Tell the backend which conversation the user is looking at, or `null` for
 * none.
 *
 * The backend narrows its live subscription from "every conversation on this
 * host" to that one — the `selective_events_v1` lane, which is most of the
 * traffic on a host that is streaming a reply. A hint rather than a command: the
 * wide lane is what runs when nothing is set, when the host did not confirm the
 * capability, or when the id is not a plain token, so a failure here costs bytes
 * and nothing else.
 */
export async function setRemotePeerVisibleSession(
  desktopId: string,
  sessionId: string | null,
): Promise<void> {
  try {
    await invokeCommand<void>("remote_peer_set_visible_session", { desktopId, sessionId });
  }
  catch {
    // Deliberately swallowed, and the only place in this module that does it:
    // no view may break because a bandwidth hint could not be delivered, and the
    // fallback (the wide lane) is what was running before this call existed —
    // including on a backend that predates it.
  }
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
        // The workspace's *name*, for the merged list's group heading. Carried
        // explicitly like every other field: this parser is field-by-field, so an
        // optional field the host sends reaches the UI only if it is named here
        // (the heading showed the fallback text until this line existed).
        ...(typeof row.workspaceName === "string" && row.workspaceName
          ? { workspaceName: row.workspaceName }
          : {}),
        // Whether that workspace is pinned on the host. Carried like every other
        // field — this parser is field-by-field, so a host field the parser does
        // not name never reaches the merged list, and the group would never lead.
        ...(typeof row.workspacePinned === "boolean" ? { workspacePinned: row.workspacePinned } : {}),
        // When that workspace itself was last touched on that machine: the key
        // the host orders its own workspace list by, and therefore the group's
        // place here. Named here for the same reason as the two above.
        ...(typeof row.workspaceOrderedAt === "number" ? { workspaceOrderedAt: row.workspaceOrderedAt } : {}),
        // The conversation this one was forked from, on that host. Without it a
        // remote fork is drawn as a root — no indent and no fold toggle — while
        // the same fork on this machine has both.
        ...(typeof row.parentSessionId === "string" && row.parentSessionId
          ? { parentSessionId: row.parentSessionId }
          : {}),
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
 *
 * `attachments` are ids of files already uploaded to that host. They travel as
 * references rather than as bytes: the bytes are already on the other machine,
 * and re-sending them would be a second copy that could disagree with the first.
 */
export async function promptRemoteConversation(
  desktopId: string,
  sessionId: string,
  message: string,
  attachments: string[] = [],
  /**
   * The workspace to create the conversation in, for a draft.
   *
   * The host's own `workspace_id`, not a path: it resolves the id against its
   * own store, and this machine has no way to name a directory there. Sent only
   * when the conversation is being created — an existing one is already in a
   * workspace, and the host ignores it.
   */
  workspaceId?: string | null,
  /**
   * The model and thinking level a *new* conversation should be created with.
   *
   * Sent for a draft so that what the picker showed is what runs: a conversation
   * whose model was chosen before it existed has no other way to carry that
   * choice, and leaving it out means the host picks, which may not be what the
   * reader was looking at. An existing conversation carries neither — its model
   * and level are its own, already set on the host by `set_model`.
   */
  fresh?: { modelId?: string | null; thinkingLevel?: string | null },
  /**
   * Stable command id the host keys its durable receipt on. When a prompt is
   * retried after a transport failure, reusing the same id lets the host return
   * the cached ack instead of running the prompt twice.
   */
  commandId?: string,
): Promise<RemoteConversationIds> {
  const provider = !sessionId && fresh?.modelId ? remoteModelProvider(fresh.modelId) : null;
  const ack = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    {
      type: "prompt",
      ...(commandId ? { id: commandId } : {}),
      sessionId,
      message,
      ...(!sessionId && workspaceId ? { mode: "workspace", workspaceId } : {}),
      ...(!sessionId && fresh?.modelId
        ? { modelId: fresh.modelId, ...(provider ? { providerId: provider } : {}) }
        : {}),
      ...(!sessionId && fresh?.thinkingLevel ? { level: fresh.thinkingLevel } : {}),
      ...(attachments.length > 0
        ? { attachments: attachments.map(uploadId => ({ uploadId })) }
        : {}),
    },
    sessionId || "new",
  );
  const created = ack?.sessionId;
  if (typeof created !== "string" || !created)
    throw new Error("remote_prompt_without_session_id");
  return { sessionId: created, threadId: typeof ack.threadId === "string" ? ack.threadId : "" };
}

/**
 * Ask the host whether a prompt or continuation it already accepted.
 *
 * The host keys its durable receipt on the command `id`; a retry of a delivery
 * can check this before re-sending and avoid running the prompt twice.
 */
export async function getRemotePromptReceipt(
  desktopId: string,
  commandId: string,
): Promise<{ sessionId: string; threadId: string; runId: string } | null> {
  const ack = await requestRemotePeer<Record<string, unknown> | null>(
    desktopId,
    { type: "get_prompt_receipt", promptId: commandId },
    "list",
  );
  if (
    !ack
    || typeof ack !== "object"
    || typeof ack.sessionId !== "string"
    || !ack.sessionId
    || typeof ack.runId !== "string"
    || !ack.runId
  ) {
    return null;
  }
  return {
    sessionId: ack.sessionId,
    threadId: typeof ack.threadId === "string" ? ack.threadId : "",
    runId: ack.runId,
  };
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
 * A file pulled from a host to preview in place, as this machine now has it.
 *
 * `path` is a *local* path into this app's preview cache — that is what the
 * overlay's readers open — and `name` is the host's own name, which is what the
 * overlay shows and what its renderer is chosen from.
 */
export interface RemotePreviewFile {
  name: string;
  path: string;
}

/**
 * Fetch a file from a host so this app can show it, without a save dialog.
 *
 * The destination is this app's own cache rather than a path the user picks: a
 * preview is a look at a file, not a copy of it, and asking where to put a copy
 * every time someone clicks a file in a panel is the wrong question. Saving a
 * copy is still `downloadRemoteFile`, behind its own control.
 */
export function previewRemoteFile(input: {
  desktopId: string;
  sessionId: string;
  path: string;
  name: string;
}): Promise<RemotePreviewFile> {
  return invokeCommand<RemotePreviewFile>("remote_peer_preview_file", {
    desktopId: input.desktopId,
    sessionId: input.sessionId,
    path: input.path,
    name: input.name,
  });
}

/**
 * A file this machine has handed to a host, as the composer needs it.
 *
 * `uploadId` is what a prompt attaches; `name` is the host's display name for
 * it, which is what the chip shows.
 */
export interface RemoteUpload {
  uploadId: string;
  name: string;
  contentHash: string;
}

/**
 * Send a local file to a host, and return the reference a prompt attaches.
 *
 * Nothing is written into the other machine's filesystem: the host stages the
 * bytes and a message claims them, so an upload that is never sent is discarded
 * on its own.
 */
export function uploadRemoteFile(input: {
  desktopId: string;
  path: string;
  name?: string;
}): Promise<RemoteUpload> {
  return invokeCommand<RemoteUpload>("remote_peer_upload_file", {
    desktopId: input.desktopId,
    path: input.path,
    ...(input.name === undefined ? {} : { name: input.name }),
  });
}

/**
 * One model in a host's catalogue.
 *
 * `id` is provider-local: the same id can exist under two providers, which is
 * why a reference (below) is what identifies a choice.
 */
export interface RemoteModelInfo {
  id: string;
  label?: string;
  provider?: string;
  isDefault?: boolean;
  /**
   * Whether the model accepts thinking controls. Carried because the task form
   * asks it — a model that does not support thinking must not be offered a
   * thinking level, and "unknown" (`undefined`) is not the same as `false`.
   */
  reasoning?: boolean | null;
  thinkingLevel?: string | null;
}

/**
 * The stable reference for a model: `provider/id` where there is a provider.
 *
 * Mirrors the phone's rule, and the host's own `qualified_model_id`: a catalogue
 * id is provider-local even when it happens to contain a slash (there are models
 * literally named `openrouter/auto`), so the provider is prefixed rather than
 * inferred from the id.
 */
export function remoteModelReference(model: Pick<RemoteModelInfo, "id" | "provider">): string {
  return model.provider ? `${model.provider}/${model.id}` : model.id;
}

/** The provider part of a reference, for the host's two-field `set_model`. */
export function remoteModelProvider(reference: string): string | undefined {
  const separator = reference.indexOf("/");
  return separator > 0 ? reference.slice(0, separator) : undefined;
}

/** The host's model catalogue, as this machine may choose from. */
export async function listRemoteModels(desktopId: string): Promise<RemoteModelInfo[]> {
  return parseModelList(await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "list_models" },
    "list",
  ));
}

/**
 * The host's models as its *own settings page* sees them: **including** the ones
 * the user has hidden.
 *
 * A different command from `listRemoteModels`, and the difference is the whole
 * point of the settings page. The host answers `list_models` with the models it
 * currently offers — it strips `hiddenModels` before replying — so a visibility
 * switch fed from that list could only ever show models that are already on, and
 * a hidden model could never be switched back on. `list_settings_models` is the
 * unfiltered catalogue, which is what a page of switches needs (and what the
 * phone's models page reads, for the same reason).
 */
export async function listRemoteSettingsModels(desktopId: string): Promise<RemoteModelInfo[]> {
  return parseModelList(await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "list_settings_models" },
    "list",
  ));
}

/** One entry of a host's model list, as the two read commands both spell it. */
function parseModelList(raw: Record<string, unknown> | null | undefined): RemoteModelInfo[] {
  const models = Array.isArray(raw?.models) ? raw.models : [];
  return models.flatMap((entry): RemoteModelInfo[] => {
    if (typeof entry !== "object" || entry === null)
      return [];
    const row = entry as Record<string, unknown>;
    if (typeof row.id !== "string" || !row.id)
      return [];
    return [{
      id: row.id,
      ...(typeof row.label === "string" ? { label: row.label } : {}),
      ...(typeof row.provider === "string" ? { provider: row.provider } : {}),
      ...(row.isDefault === true ? { isDefault: true } : {}),
      ...(typeof row.reasoning === "boolean" || row.reasoning === null ? { reasoning: row.reasoning as boolean | null } : {}),
      ...(typeof row.thinkingLevel === "string" ? { thinkingLevel: row.thinkingLevel } : {}),
    }];
  });
}

/**
 * A host's models as this app's model options.
 *
 * The two shapes are the same catalogue in different wrappers: the host sends
 * the agent's own list, so a shared model id resolves to the same model on
 * either side. The provider is prefixed rather than inferred, because a
 * catalogue id may itself contain a slash.
 */
export function remoteAgentModelOptions(models: RemoteModelInfo[]): AgentModelOption[] {
  return models.map(model => ({
    id: remoteModelReference(model),
    label: model.label ?? model.id,
    provider: model.provider ?? "",
    ...(model.reasoning === undefined ? {} : { reasoning: model.reasoning }),
    ...(model.thinkingLevel === undefined ? {} : { thinkingLevel: model.thinkingLevel }),
    ...(model.isDefault === undefined ? {} : { isDefault: model.isDefault }),
  }));
}

/** What a host reports about one of its conversations, as this machine may show it. */
export interface RemoteConversationSettings {
  /** The agent's model reference, or `null` when it did not say. */
  model: string | null;
  thinkingLevel: string | null;
}

/**
 * A conversation's settings, read out of a host's `get_state` reply.
 *
 * Exported because the reply has two readers: this module's own read, and the
 * remote timeline, which reads the *same* command for the run in flight and hands
 * its reply over rather than each hook asking the host again (see
 * `RemoteSessionStateReading`).
 */
export function parseRemoteConversationSettings(
  raw: unknown,
): RemoteConversationSettings {
  const record = typeof raw === "object" && raw !== null ? raw as Record<string, unknown> : {};
  return {
    model: typeof record.model === "string" && record.model ? record.model : null,
    thinkingLevel:
      typeof record.thinkingLevel === "string" && record.thinkingLevel ? record.thinkingLevel : null,
  };
}

/**
 * Read a conversation's model and thinking level.
 *
 * Their own read rather than part of the timeline: these are settings the host
 * owns and can change from its own screen, so they are fetched separately and
 * refreshed when it says they moved.
 */
export async function getRemoteConversationSettings(
  desktopId: string,
  sessionId: string,
): Promise<RemoteConversationSettings> {
  const raw = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "get_state", sessionId },
    sessionId,
  );
  return parseRemoteConversationSettings(raw);
}

/** A host's own app settings, as far as this app reads them. */
export interface RemoteDesktopSettings {
  /**
   * Whether that machine wants its conversations to get skill recommendations.
   *
   * A *host* setting, not one of this machine's: it is what decides whether a card
   * is offered for a conversation that lives there.
   */
  skillRecommend: boolean;
}

/**
 * Read a host's own app settings.
 *
 * One command for the whole machine rather than for one of its conversations, so
 * it is read once per connection and dropped when the host says it moved
 * (`app_settings_changed`) — see `remoteHostCatalog`.
 */
export async function getRemoteDesktopSettings(desktopId: string): Promise<RemoteDesktopSettings> {
  const raw = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "get_desktop_settings" },
    "list",
  );
  return { skillRecommend: raw?.skillRecommend === true };
}

/**
 * Change a conversation's model, on the host that owns it.
 *
 * The request carries the reference *and* its provider because the host joins
 * them itself; sending a bare id would be resolved against whatever provider it
 * defaulted to, which is how the same id under two providers becomes the wrong
 * model.
 */
export function setRemoteConversationModel(
  desktopId: string,
  sessionId: string,
  reference: string,
): Promise<unknown> {
  const provider = remoteModelProvider(reference);
  return requestRemotePeer(
    desktopId,
    {
      type: "set_model",
      sessionId,
      modelId: reference,
      ...(provider ? { providerId: provider } : {}),
    },
    sessionId,
  );
}

/** Change a conversation's thinking level, on the host that owns it. */
export function setRemoteConversationThinkingLevel(
  desktopId: string,
  sessionId: string,
  level: string,
): Promise<unknown> {
  return requestRemotePeer(desktopId, { type: "set_thinking_level", sessionId, level }, sessionId);
}

/**
 * Resume a run that failed, on the host that stopped it.
 *
 * Only a *failed* run can be continued — that is the host's own precondition,
 * and it is why this is offered on a row that reports an error rather than on
 * any row. Resuming re-runs the same turn with whatever it already had, which is
 * why it is the right answer to a failure: nothing has to be re-sent, and an
 * attachment the turn carried is still there.
 */
export function continueRemoteRun(
  desktopId: string,
  sessionId: string,
  runId: string,
  /**
   * Stable command id the host keys its durable receipt on. When a continuation
   * is retried after a transport failure, reusing the same id lets the host return
   * the cached ack instead of running the continuation twice.
   */
  commandId?: string,
): Promise<unknown> {
  return requestRemotePeer(desktopId, {
    type: "continue_run",
    sessionId,
    runId,
    ...(commandId ? { id: commandId } : {}),
  }, sessionId);
}

/**
 * One skill a host has installed.
 *
 * Names are localized on that host's catalogue rather than in the app's own
 * bundle, so both languages travel and the caller picks.
 */
export interface RemoteSkillInfo {
  id: string;
  name: string;
  description: string;
  nameZh?: string | null;
  descriptionZh?: string | null;
  version?: string | null;
  /** The version a *catalogue* would install; only the installable list carries it. */
  latestVersion?: string | null;
}

/** One skill that host could install. */
export interface RemoteAvailableSkill {
  id: string;
  name: string;
  description: string;
  nameZh?: string | null;
  descriptionZh?: string | null;
  latestVersion?: string | null;
}

/**
 * Pick the name and description for a language.
 *
 * A catalogue entry without a translation falls back to the other language
 * rather than to nothing: the user is deciding whether to install it, and a blank
 * row tells them nothing. A pure function so both fallbacks are testable without
 * a host.
 */
export function localizedSkill(
  skill: RemoteSkillInfo | RemoteAvailableSkill,
  language: string,
): { name: string; description: string } {
  const chinese = language.startsWith("zh");
  const name = chinese ? skill.nameZh || skill.name : skill.name || skill.nameZh;
  const description = chinese
    ? skill.descriptionZh || skill.description
    : skill.description || skill.descriptionZh;
  return { name: name || skill.id, description: description || "" };
}

function skillRows(raw: unknown): RemoteSkillInfo[] {
  const list = Array.isArray((raw as Record<string, unknown> | null)?.skills)
    ? (raw as { skills: unknown[] }).skills
    : [];
  return list.flatMap((entry): RemoteSkillInfo[] => {
    if (typeof entry !== "object" || entry === null)
      return [];
    const row = entry as Record<string, unknown>;
    if (typeof row.id !== "string" || !row.id)
      return [];
    return [{
      id: row.id,
      name: typeof row.name === "string" ? row.name : row.id,
      description: typeof row.description === "string" ? row.description : "",
      ...(typeof row.nameZh === "string" ? { nameZh: row.nameZh } : {}),
      ...(typeof row.descriptionZh === "string" ? { descriptionZh: row.descriptionZh } : {}),
      ...(typeof row.version === "string" ? { version: row.version } : {}),
      ...(typeof row.latestVersion === "string" ? { latestVersion: row.latestVersion } : {}),
    }];
  });
}

/** The skills that host already has. */
export async function listRemoteInstalledSkills(desktopId: string): Promise<RemoteSkillInfo[]> {
  return skillRows(await requestRemotePeer(desktopId, { type: "list_skills" }, "list"));
}

/** The skills that host could install. */
export async function listRemoteAvailableSkills(desktopId: string): Promise<RemoteSkillInfo[]> {
  return skillRows(await requestRemotePeer(desktopId, { type: "list_available_skills" }, "list"));
}

/**
 * Install a skill on that host.
 *
 * The version is the catalogue's `latestVersion`, which is what the host expects
 * to resolve; asking for a version it does not publish is refused there.
 */
export function installRemoteSkill(
  desktopId: string,
  skillId: string,
  version: string,
): Promise<unknown> {
  return requestRemotePeer(desktopId, { type: "install_skill", skillId, version }, "list");
}

/**
 * Remove a skill from that host, and report whether anything was.
 *
 * The host answers `removed: false` for a skill that was already gone — which is
 * not an error: two clients can be looking at the same list, and the second one
 * asking to remove what the first already removed has its answer.
 */
export async function uninstallRemoteSkill(
  desktopId: string,
  skillId: string,
): Promise<boolean> {
  const raw = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "uninstall_skill", skillId },
    "list",
  );
  return raw?.removed === true;
}

/**
 * The approval mode that host is running under, and whether its sandbox exists.
 *
 * The two arrive together because the second decides which modes make sense: a
 * host without a sandbox cannot offer `sandbox` or `auto`, and offering them
 * would only let the user pick something the host would refuse.
 */
export async function getRemoteApprovalSettings(
  desktopId: string,
): Promise<{ approvalTier: string; sandboxAvailable: boolean }> {
  const raw = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "get_settings" },
    "list",
  );
  return {
    approvalTier: typeof raw?.approvalTier === "string" ? raw.approvalTier : "manual",
    sandboxAvailable: raw?.sandboxAvailable === true,
  };
}

/**
 * Set that host's approval mode.
 *
 * The host answers with the tier it actually settled on, which is why this
 * returns it rather than echoing the request: `sandbox` on a host whose sandbox
 * turned out to be missing is answered with `manual`, and a UI that showed what
 * it asked for would be reporting a mode the host is not in.
 */
export async function setRemoteApprovalTier(
  desktopId: string,
  tier: string,
): Promise<string> {
  const raw = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "set_approval_tier", tier },
    "list",
  );
  return typeof raw?.approvalTier === "string" ? raw.approvalTier : tier;
}

/** One workspace on a host, as this machine may show it. */
export interface RemoteWorkspaceRow {
  id: string;
  name: string;
  path: string;
  pinned: boolean;
}

/**
 * That host's workspaces.
 *
 * A workspace is a directory on the host, so its path is meaningful only there —
 * which is why this list is about naming and ordering that machine's folders,
 * not about reaching into them.
 */
export async function listRemoteWorkspaces(desktopId: string): Promise<RemoteWorkspaceRow[]> {
  const raw = await invokeCommand<{ workspaces?: unknown[] }>("remote_peer_workspaces", { desktopId });
  return workspaceRows(raw?.workspaces);
}

function workspaceRows(items: unknown): RemoteWorkspaceRow[] {
  if (!Array.isArray(items))
    return [];
  return items.flatMap((entry): RemoteWorkspaceRow[] => {
    if (typeof entry !== "object" || entry === null)
      return [];
    const row = entry as Record<string, unknown>;
    if (typeof row.id !== "string" || !row.id)
      return [];
    return [{
      id: row.id,
      name: typeof row.name === "string" && row.name ? row.name : (typeof row.path === "string" ? row.path : row.id),
      path: typeof row.path === "string" ? row.path : "",
      pinned: row.pinned === true,
    }];
  });
}

/**
 * Register an existing directory on that host as a workspace.
 *
 * Typed by hand because this machine has no folder picker for the host's disk,
 * and the directory must already exist there — the same rule the host's own
 * create-workspace form follows.
 */
export async function createRemoteWorkspace(
  desktopId: string,
  path: string,
  name: string,
): Promise<RemoteWorkspaceRow> {
  const raw = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "create_workspace", path: path.trim(), name: name.trim() },
    "list",
  );
  const [created] = workspaceRows([raw?.workspace]);
  if (!created)
    throw new Error("invalid_workspace_snapshot");
  return created;
}

/** Pin or unpin one of that host's workspaces. */
export async function setRemoteWorkspacePinned(
  desktopId: string,
  workspaceId: string,
  pinned: boolean,
): Promise<void> {
  await requestRemotePeer(desktopId, { type: "set_workspace_pinned", workspaceId, pinned }, "list");
}

/**
 * Rename one of that host's workspaces.
 *
 * The name lives in the host's store and nowhere else — a workspace is a
 * directory there, and this is its label, so nothing is moved and no path
 * changes. The host applies its own rules to the name (it trims, and refuses an
 * empty one), which is why the refusal arrives as a rejection rather than being
 * second-guessed here.
 */
export async function renameRemoteWorkspace(
  desktopId: string,
  workspaceId: string,
  name: string,
): Promise<unknown> {
  return requestRemotePeer(
    desktopId,
    { type: "rename_workspace", workspaceId, name },
    "list",
  );
}

/**
 * Remove a workspace from that host.
 *
 * The host removes its conversations and threads with it, so this is not a
 * cosmetic delete — which is why the panel confirms before asking.
 */
export async function deleteRemoteWorkspace(
  desktopId: string,
  workspaceId: string,
): Promise<void> {
  await requestRemotePeer(desktopId, { type: "delete_workspace", workspaceId }, "list");
}

/**
 * Ask that host to name one of its conversations.
 *
 * The host runs this with the conversation's own model, and answers the title it
 * produced. The title is returned rather than written: naming the conversation
 * is still the user's decision, exactly as it is for one of this app's own.
 */
export async function generateRemoteTitle(
  desktopId: string,
  sessionId: string,
  language: string,
): Promise<string> {
  const raw = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "generate_session_title", sessionId, mode: language },
    sessionId,
  );
  const title = typeof raw?.title === "string" ? raw.title.trim() : "";
  if (!title)
    throw new Error("empty_title");
  return title;
}

/**
 * This host's providers, as its own settings page sees them.
 *
 * No cache, unlike this app's own provider reads: the two views are different
 * machines' configurations, and one shared cache would show one machine's
 * providers under the other's name.
 */
export async function listRemoteProviders(desktopId: string): Promise<ProvidersView> {
  return await requestRemotePeer<ProvidersView>(desktopId, { type: "list_providers" }, "list");
}

/**
 * Write one built-in provider's key and/or base URL on that host.
 *
 * `updateApiKey` is what distinguishes "leave the key alone" from "set it": the
 * host treats an absent key as no change, so clearing one has to say so
 * explicitly with `apiKey: null`.
 */
export async function updateRemoteBuiltinProvider(
  desktopId: string,
  input: { id: string; baseUrl?: string; apiKey?: string | null; updateApiKey: boolean },
): Promise<ProvidersView> {
  return await requestRemotePeer<ProvidersView>(
    desktopId,
    { type: "update_builtin_provider", provider: input },
    "list",
  );
}

/** Add or edit one custom provider on that host. */
export async function upsertRemoteCustomProvider(
  desktopId: string,
  input: {
    id: string;
    name: string;
    api: string;
    baseUrl: string;
    apiKey?: string | null;
    models: unknown[];
    create: boolean;
  },
): Promise<ProvidersView> {
  return await requestRemotePeer<ProvidersView>(
    desktopId,
    { type: "upsert_custom_provider", provider: input },
    "list",
  );
}

/** Remove one custom provider from that host. */
export async function deleteRemoteCustomProvider(
  desktopId: string,
  providerId: string,
): Promise<ProvidersView> {
  return await requestRemotePeer<ProvidersView>(
    desktopId,
    { type: "delete_custom_provider", providerId },
    "list",
  );
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
