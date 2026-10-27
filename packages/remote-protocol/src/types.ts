export interface PairingCode {
  v: 2;
  nonce: string;
  claim_url: string;
  exp: number;
  /**
   * Present when the invitation was minted by a self-hosted relay instead of
   * the Future platform: the origin the claim endpoint belongs to. The payload
   * keeps `v: 2` because its *shape* did not change; the invitation URL says
   * `v=3`, which is what a v2-only client refuses.
   */
  relay?: string;
}


export interface RemoteSession {
  sessionId: string;
  threadId: string;
  title: string;
  mode?: "chat" | "workspace";
  workspaceId?: string;
  /** Agent lineage, omitted by older desktops. */
  parentSessionId?: string | null;
  /** Pinned to the top of the session list (desktop `threads.pinned`). */
  pinned?: boolean;
  streaming: boolean;
  status?: string;
  /**
   * The name of the workspace this conversation sits in, as its own machine
   * calls it. Sent per session row (a client grouping a merged list needs the
   * name for its heading), omitted by a desktop older than the field.
   */
  workspaceName?: string | null;
  /** Whether that workspace is pinned *on its own machine*. */
  workspacePinned?: boolean | null;
  /** When that workspace itself was last touched on its own machine. */
  workspaceOrderedAt?: number | null;
  /**
   * Last activity (unix millis), from the desktop's own
   * `COALESCE(last_message_at, updated_at, created_at)`. Absent on a desktop
   * older than the field, in which case the session sorts below dated ones
   * rather than pretending to be current.
   */
  lastMessageAt?: number | null;
  /**
   * When the conversation was started (unix millis), from `threads.created_at`.
   *
   * A run rewrites `lastMessageAt` about once a second while it streams, so a
   * client that ranks the *running* conversations by that reshuffles the list
   * under the reader's thumb; the creation time does not move. Absent on a
   * desktop older than the field, and the client falls back to recency there.
   */
  createdAt?: number | null;
}

export interface RemoteWorkspace {
  id: string;
  name: string;
  path: string;
  description?: string;
  /** Pinned to the top of the workspace list (desktop `workspaces.pinned`). */
  pinned?: boolean;
}

export interface PresenceSession {
  sessionId: string;
  threadId: string;
  title: string;
  mode?: "chat" | "workspace";
  workspaceId?: string;
  parentSessionId?: string | null;
  /** Desktop `threads.pinned` — hoisted to the top of the session list. */
  pinned?: boolean;
  streaming: boolean;
  status?: string;
  /** The workspace's own name, pin and place, as the host reports them per row. */
  workspaceName?: string | null;
  workspacePinned?: boolean | null;
  workspaceOrderedAt?: number | null;
  /** Same wire field as `RemoteSession.createdAt`: when the conversation began. */
  createdAt?: number | null;
}

export interface SnapshotVersion {
  epoch: string;
  revision: number;
}

/**
 * One row of a catalog activity packet: the session, and the activity
 * timestamp it now has.
 *
 * A streaming run rewrites its conversation's activity timestamp about once a
 * second, and that value is the list's ordering key. Sending the whole session
 * list for it cost ~95 KB per second on a desktop with a few hundred
 * conversations; this packet carries only the rows that moved.
 */
export interface SessionActivity {
  sessionId: string;
  lastMessageAt: number;
}

/** Per-domain catalog revisions advertised on a presence heartbeat. */
export interface CatalogRevisions {
  epoch?: string;
  sessions?: number;
  workspaces?: number;
}

export interface Presence {
  catalogEpoch?: string;
  sessionsVersion?: SnapshotVersion;
  workspacesVersion?: SnapshotVersion;
  /**
   * Catalog revisions as of this heartbeat. A revision newer than the one this
   * client applied proves a pushed snapshot was lost (core NATS is
   * at-most-once), so the client pulls the catalogue itself. The desktop no
   * longer re-sends an unchanged snapshot on a timer, which is what makes an
   * idle directory cost only this packet.
   */
  catalogVersion?: CatalogRevisions;
  agentAvailable?: boolean;
  online: boolean;
  /** An intentional desktop disconnect; `online: false` is authoritative. */
  disconnected?: boolean;
  /** Why Desktop explicitly ended the remote session. */
  reason?: string;
  /** Immediate desktop-originated unpair notice; server revocation backs it up. */
  unpaired?: boolean;
  pairId: string;
  bridgeInstanceId: string;
  /**
   * Whether this device may drive the desktop right now.
   *
   * `standby` means another paired device holds it. Additive on the wire: a
   * desktop that predates this field sends nothing, and an absent value is read
   * as "in control" — that is what every older desktop meant by being connected.
   */
  control?: "active" | "standby";
  /**
   * Who holds the desktop, when it is not this device.
   *
   * This is what lets the phone ask "taking over disconnects <name>?" instead of
   * either stealing the machine silently or refusing without saying why. A
   * holder that never declared a name is described neutrally.
   */
  holder?: { pairId?: string | null; name?: string; kind?: string } | null;
  lastHeartbeatTs: number;
  sessions?: PresenceSession[];
  workspaces?: RemoteWorkspace[];
}


/** Installed skills discovered by the connected Agent (same catalogue as desktop). */
export interface RemoteSkill {
  name: string;
  description: string;
  nameZh?: string | null;
  descriptionZh?: string | null;
}

/** Authoritative desktop preferences. Never write these to phone storage. */
export interface DesktopSettings {
  autoUpgradeSkills: boolean;
  autoTitleFirstTurn: boolean;
  autoConnectRemote: boolean;
  hiddenModels: string[];
  /**
   * Offer a fitting skill before sending a message (PRD v1.6 §3). On by
   * default, so an older desktop that omits it must not read as "off".
   */
  skillRecommend?: boolean;
}

/**
 * A catalog provider on the desktop. `hasApiKey` is all the phone ever learns
 * about the credential: keys are write-only from here (set or cleared, never
 * read back).
 */
export interface RemoteBuiltinProvider {
  id: string;
  name: string;
  baseUrl: string;
  hasApiKey: boolean;
  modelCount: number;
  /** The catalog base URL is a placeholder; the user must supply their own. */
  requiresBaseUrl: boolean;
}

/** One model of a custom provider. Absent fields mean the desktop default. */
export interface RemoteProviderModel {
  id: string;
  name?: string;
  supportsImages?: boolean;
  /**
   * Kept for the wire (an older desktop still reads it) but it no longer gates
   * anything: the levels below decide whether the model has thinking controls.
   */
  reasoning?: boolean;
  /**
   * The thinking levels this model accepts, in display order — each entry a wire
   * value sent verbatim. `"default"` is implicit (an empty list means only
   * `default`) and is never stored here. Absent on an older host, which also
   * ignores it on an upsert.
   */
  thinkingLevels?: string[];
  contextWindow?: number;
  maxTokens?: number;
  /** Per-1M-token prices; 0 means unpriced (adds nothing to the amount). */
  inputCost?: number;
  outputCost?: number;
  cacheReadCost?: number;
  cacheWriteCost?: number;
}

export interface RemoteCustomProvider {
  id: string;
  name: string;
  api: string;
  baseUrl: string;
  hasApiKey: boolean;
  models: RemoteProviderModel[];
}

/** API shapes a custom provider may speak (mirrors the desktop's choices). */
export const CUSTOM_PROVIDER_APIS = ["openai-completions", "openai-responses", "anthropic"] as const;

export interface ProvidersView {
  builtin: RemoteBuiltinProvider[];
  custom: RemoteCustomProvider[];
}

/** The write payload of one custom provider (mirrors UpsertCustomProviderInput). */
export interface CustomProviderUpsert {
  id: string;
  name: string;
  api: string;
  baseUrl: string;
  /** Absent keeps the stored key; the desktop writes it only when non-empty. */
  apiKey?: string | null;
  models: RemoteProviderModel[];
  create: boolean;
}

/**
 * One atomic built-in provider write (mirrors UpdateBuiltinProviderInput):
 * `updateApiKey` decides whether `apiKey` is applied at all, and a null/empty
 * `apiKey` then clears the stored credential.
 */
export interface BuiltinProviderUpdate {
  id: string;
  baseUrl?: string;
  apiKey?: string | null;
  updateApiKey: boolean;
}

export interface InstalledSkill extends RemoteSkill {
  id: string;
  version?: string | null;
}

export interface AvailableSkill {
  id: string;
  name: string;
  description: string;
  nameZh?: string;
  descriptionZh?: string;
  latestVersion?: string | null;
}

export interface RemoteModel {
  id: string;
  label?: string;
  provider?: string;
  isDefault?: boolean;
  /** Whether the model's input modality accepts images (agent catalogue). */
  supportsImages?: boolean;
  /**
   * The levels this model offers, in display order. The host sends the list
   * already resolved: always non-empty, always starting with `"default"`, and
   * every other entry is a wire value sent verbatim (`"off"` disables thinking
   * explicitly; `"max"` and arbitrary provider tokens are allowed).
   *
   * Absent on an older host: treat that model as offering the legacy
   * [`THINKING_LEVELS`] and starting at `"medium"` — never send the literal
   * `"default"` to a host that predates the field.
   */
  thinkingLevels?: string[] | null;
  /**
   * The level a conversation starts at on this model, already clamped to
   * `thinkingLevels` by the host. `"default"` for a current host; unset on an
   * older one.
   */
  defaultThinkingLevel?: string | null;
}

/** Stable agent model identifier; model ids are only unique within a provider. */
export function modelReference(model: Pick<RemoteModel, "id" | "provider">): string {
  // Catalogue ids are raw provider-local ids, even when they contain '/' or
  // already begin with the provider's name (for example openrouter/auto).
  return model.provider ? `${model.provider}/${model.id}` : model.id;
}

export function modelProviderFromReference(modelReference: string): string | undefined {
  const separator = modelReference.indexOf("/");
  return separator > 0 ? modelReference.slice(0, separator) : undefined;
}

/**
 * Session-level token usage and the amount it represents, mirroring the
 * agent's get_state `usage` object. The per-category amounts are priced by the
 * agent from the model's rates; a model with no rates on file reports zeros.
 */
export interface RemoteSessionUsage {
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  /** Total spent (¥): the provider's own billing, or the priced estimate. */
  costCny: number;
  costInputCny: number;
  costOutputCny: number;
  costCacheReadCny: number;
  costCacheWriteCny: number;
}

export interface RemoteSessionState {
  isCompacting?: boolean;
  model?: string;
  thinkingLevel?: ThinkingLevel;
  /** Token usage + amount for the session; absent until the agent reports it. */
  usage?: RemoteSessionUsage | null;
  /** The session's active (in-flight) run, if any — the tail of its events
   *  can be backfilled from `get_events_since` to resync on open/reconnect. */
  activeRun?: { runId?: string } | null;
  /**
   * The run the session's journal records as never having ended, reported only
   * when no run is live (the agent crashed or restarted mid-run). It is the
   * host's answer to "this reply's log has no terminal, and nobody is working
   * on it" — the one thing that tells a dead run apart from one still being
   * written, which the log alone cannot say. An older host omits it.
   */
  interruptedRun?: { runId?: string } | null;
}

/** A level every client has a label for. The wire accepts any other token too. */
export type KnownThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max";

/**
 * A model's thinking level as it travels the wire: the host declares the values
 * it accepts and the client sends one back verbatim, so this is deliberately an
 * open string rather than the known union.
 */
export type ThinkingLevel = string;

/** The implicit "send no thinking parameter" level, always available on a current host. */
export const DEFAULT_THINKING_LEVEL = "default";

/**
 * The vocabulary and starting level of a host that predates `thinkingLevels`.
 *
 * Only a fallback: a current host declares its own list (which always starts
 * with `"default"`), so this is what an older host's models offer and where its
 * conversations start. It is also the never-send rule: `"default"` is not in it.
 */
export const THINKING_LEVELS: readonly KnownThinkingLevel[] = ["off", "minimal", "low", "medium", "high", "xhigh"];

/** Where an older host's conversations start (its model list has no `"default"`). */
export const LEGACY_DEFAULT_THINKING_LEVEL = "medium";

/**
 * The levels a model offers, in the host's display order.
 *
 * The host already resolved the list, so it is returned as-is — arbitrary wire
 * values (`"max"`, a provider-specific token) must reach the picker, not be
 * filtered down to a client-side vocabulary. Absent (or empty, which a current
 * host never sends) means an older host: the legacy six, unchanged behaviour.
 */
export function modelThinkingLevels(
  model: Pick<RemoteModel, "thinkingLevels"> | undefined,
): readonly ThinkingLevel[] {
  const declared = model?.thinkingLevels;
  return declared && declared.length > 0 ? declared : THINKING_LEVELS;
}

/**
 * The level a conversation starts at on `model` when none is chosen.
 *
 * A current host declares it (`defaultThinkingLevel`, `"default"` in practice).
 * An older host has no such list, so it keeps its legacy `"medium"` — sending
 * `"default"` there would be a value it never offered.
 */
export function defaultThinkingLevelForModel(
  model: Pick<RemoteModel, "thinkingLevels" | "defaultThinkingLevel"> | undefined,
): ThinkingLevel {
  const declared = model?.thinkingLevels;
  if (!declared || declared.length === 0)
    return LEGACY_DEFAULT_THINKING_LEVEL;
  const stated = model?.defaultThinkingLevel;
  if (stated && declared.includes(stated))
    return stated;
  return declared.includes(DEFAULT_THINKING_LEVEL) ? DEFAULT_THINKING_LEVEL : declared[0]!;
}

/**
 * The level to show for a conversation on `model`.
 *
 * A stored level the model no longer offers is not a level this conversation can
 * run at — the host clamps it — so it is replaced by the model's starting level
 * rather than displayed as the current effort. An unknown model keeps the caller's
 * own value (undefined): there is no list to resolve against yet.
 */
export function resolveModelThinkingLevel(
  level: string | null | undefined,
  model: Pick<RemoteModel, "thinkingLevels" | "defaultThinkingLevel"> | undefined,
): ThinkingLevel | undefined {
  if (!model)
    return undefined;
  if (level && modelThinkingLevels(model).includes(level))
    return level;
  return defaultThinkingLevelForModel(model);
}


export interface HistoryMessage {
  role: "user" | "assistant" | "tool" | string;
  blocks: import("@future-os/thread-projection").SessionEntry["blocks"];
  metadata?: Record<string, unknown> | null;
  runId?: string | null;
}

/** Attachment chip on a user entry — mirrors the desktop `meta.attachments`. */
export interface HistoryAttachment {
  path: string;
  name: string;
  kind?: "image" | "file" | null;
  /** Local optimistic hint; durable desktop history enforces this server-side. */
  mobilePreviewUnsupported?: boolean;
}


export interface SessionFileEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
}

export interface SessionFileListing {
  rootPath: string;
  path: string;
  entries: SessionFileEntry[];
}

export interface DownloadInfo {
  transferId: string;
  name: string;
  mimeType: string;
  size: number;
  contentHash: string;
  previewKind: "image" | "markdown" | "text" | "json" | "file";
  variant: "preview" | "original";
  chunkBytes: number;
}

/**
 * Display-shaped session entry from `get_session_entries` (the agent's JSONL,
 * same source the desktop app renders). Content is plain text; user entries
 * carry attachments on `meta`, and assistant entries may carry the model's
 * thinking and the tool calls it introduced.
 */
export type HistoryEntry = import("@future-os/thread-projection").SessionEntry;

export interface StreamEvent {
  type: string;
  data: string;
  runId?: string;
  idx?: number;
  /**
   * How many source events this one carries, when the desktop merged a run's
   * text fragments (see `event_coalescing_v1`). The merged event occupies the
   * newest source index and its text is the concatenation of all of them, so a
   * client must treat `idx` as the end of a range instead of a single step.
   */
  coalescedCount?: number;
}

export interface ApprovalPayload {
  approval_request_id: string;
  tool_name?: string;
  risk_level?: string;
  kind?: string;
  title?: string;
  summary?: string;
  /** Agent-built action object (writes/paths/command) — path is surfaced to the user. */
  action?: unknown;
}


export interface RpcResponse<T = unknown> {
  success: boolean;
  data: T;
  error?: string;
}


export interface SessionsData {
  version?: SnapshotVersion;
  sessions: RemoteSession[];
}

export interface ModelsData {
  models: RemoteModel[];
  /** Desktop deliberately hid every model; unlike warm-up, this is final. */
  allModelsHidden?: boolean;
}

export interface WorkspacesData {
  version?: SnapshotVersion;
  workspaces: RemoteWorkspace[];
  /** create_workspace echoes the created row; the catalogue list omits it. */
  workspace?: RemoteWorkspace;
}

export interface HistoryData {
  messages: HistoryMessage[];
  total?: number;
  hasMore?: boolean;
  nextOffset?: number;
}

export interface EntriesData {
  entries: HistoryEntry[];
  total?: number;
  hasMore?: boolean;
  nextOffset?: number;
  /** The bridge shed whole oldest exchanges to fit its page byte budget. */
  trimmed?: boolean;
}

export interface PromptAck {
  sessionId: string;
  threadId: string;
  runId: string;
}

export interface HandshakeConfirmation {
  confirmed: boolean;
  pairId: string;
  desktopId: string;
  bridgeInstanceId: string;
  deviceId: string;
  desktopNonce: string;
  presence: Presence;
  features?: string[];
}
