export {
  PairingCode,
  RemoteSession,
  RemoteWorkspace,
  PresenceSession,
  SnapshotVersion,
  SessionActivity,
  CatalogRevisions,
  Presence,
  RemoteSkill,
  DesktopSettings,
  RemoteBuiltinProvider,
  RemoteProviderModel,
  RemoteCustomProvider,
  CUSTOM_PROVIDER_APIS,
  ProvidersView,
  CustomProviderUpsert,
  BuiltinProviderUpdate,
  InstalledSkill,
  AvailableSkill,
  RemoteModel,
  modelReference,
  modelProviderFromReference,
  modelThinkingLevels,
  THINKING_LEVELS,
  RemoteSessionUsage,
  RemoteSessionState,
  ThinkingLevel,
  HistoryMessage,
  HistoryAttachment,
  SessionFileEntry,
  SessionFileListing,
  DownloadInfo,
  HistoryEntry,
  StreamEvent,
  ApprovalPayload,
  RpcResponse,
  SessionsData,
  ModelsData,
  WorkspacesData,
  HistoryData,
  EntriesData,
  PromptAck,
  RemoteCommand,
} from "@future-os/remote-protocol";

export type ConnectionPhase =
  | "booting"
  /** Desktop explicitly ended the remote session; wait for a user retry. */
  | "stopped"
  | "unpaired"
  | "claiming"
  | "connecting"
  | "ready"
  | "reconnecting"
  | "refreshing"
  | "failed"
  | "revoked";

/**
 * A desktop this phone has paired with. Selecting one drives the connection;
 * the optional name is a local label only (see `storage.renameDesktop`).
 */
export interface PairedDesktop {
  desktopId: string;
  pairId: string;
  /** Absent until the user renames the desktop; the id is shown instead. */
  name?: string;
  /**
   * A glyph id from the fixed set (`src/remote/peerIcons.ts` here,
   * `desktop/src/features/remote-peer/peerIcons.ts` there — the same ids),
   * chosen by the user. Stored as the id rather than the glyph so the set can
   * be restyled or translated later without rewriting saved labels — and so an
   * unknown value renders the default instead of arbitrary stored text.
   */
  icon?: string;
}

export type { RemoteCredentials, TimelineItem, TimelineSegment, TimelineToolRow } from "@future-os/remote-client/types";

/**
 * A conversation the composer's draft refers to. The draft carries it as a
 * compact token (`#title`) and the message it sends carries the link
 * `[title](futureos://session/<id>)` — the id is the payload.
 */
export interface SessionReferenceTarget {
  sessionId: string;
  title: string;
}

/**
 * The references a draft holds, keyed by the token that stands in for each one.
 * Kept with the draft (see `draftStorage`) because the token alone does not
 * carry the id: losing the map would turn a reference back into plain text.
 */
export type SessionReferenceMap = Record<string, SessionReferenceTarget>;

/**
 * Terminal result of a context compaction, correlated by operation id.
 *
 * The Agent accepts a manual compaction asynchronously: the request returns an
 * operation id before the summary runs, and the outcome arrives later on the
 * session event stream. `timeout` means no terminal event was observed in time
 * — never that the operation failed.
 */
export type CompactionOutcome =
  | { status: "committed" }
  | { status: "failed"; error?: string }
  | { status: "unchanged"; alreadyCompacted: boolean; reused: boolean }
  /** No terminal event reached this client, but the session stopped compacting. */
  | { status: "unobserved" }
  | { status: "cancelled" }
  | { status: "timeout" };

export interface MobileAttachment {
  localUri: string;
  /** Original user-facing name; remains unchanged after JPEG conversion. */
  name: string;
  /** On-wire/on-disk name when converted bytes need a different extension. */
  transferName?: string;
  mimeType: string;
  kind: "image" | "file";
  /** Quota accounting always uses the original, pre-compression byte size. */
  originalSize: number;
  transferSize: number;
  /** True when localUri is an app-cache derivative that we own and may delete. */
  temporary?: boolean;
  mobilePreviewUnsupported?: boolean;
  uploadId?: string;
  contentHash?: string;
}
