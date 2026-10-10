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
): Promise<RemoteConversationIds> {
  const ack = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    {
      type: "prompt",
      sessionId,
      message,
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
  const raw = await requestRemotePeer<Record<string, unknown>>(
    desktopId,
    { type: "list_models" },
    "list",
  );
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
  return {
    model: typeof raw?.model === "string" && raw.model ? raw.model : null,
    thinkingLevel: typeof raw?.thinkingLevel === "string" && raw.thinkingLevel ? raw.thinkingLevel : null,
  };
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
): Promise<unknown> {
  return requestRemotePeer(desktopId, { type: "continue_run", sessionId, runId }, sessionId);
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
