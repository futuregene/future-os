import type { FusionSettingsPatch } from "@future-os/thread-projection";
import type { MockEntry } from "./data";
import { fusionProgressFixture } from "../../../scripts/screenshots/fixtures/fusion-progress";
/**
 * A browser-side stand-in for the desktop's Tauri (Rust) backend.
 *
 * Answers the same command names the real backend does, from an in-memory
 * dataset (`./data`), so the real frontend renders realistic content.
 *
 * Coverage follows what the shipped UI actually calls. When the app starts
 * asking for a command that is missing here, the console prints
 * `[mock] UNHANDLED COMMAND <name>` — add a handler for it and re-capture.
 */
import { applyFusionSettings, fusionSettingsModels, initialFusionSettings } from "../../../scripts/screenshots/fixtures/fusion-settings";
import fusionFixture from "../../../scripts/screenshots/fixtures/fusion.json";
import {
  appSettings,
  availableSkills,
  buildInfo,
  compactResumeEntries,
  demoFileContents,
  entriesByThread,
  gitHistoryCommits,
  gitHistoryPatch,
  HOME,
  installedSkills,
  models,
  parserApproval,
  pendingApproval,
  providersView,
  refreshedSessionUsage,
  remoteConversationState,
  remoteFileListing,
  remotePeers,
  remoteSessions,
  remoteToolArgs,
  remoteWorkspaces,
  requestedThread,
  reviewFiles,
  runs,
  sessionUsage,
  taskDeps,
  taskRuns,
  tasks,
  threads,
  toolCalls,
  toolOutputs,
  unpricedSessionUsage,
  workspaceFiles,
  workspaces,
} from "./data";
import { emit } from "./event";
import { replayPage, replayState, sessionEntries, streamRemoteRun } from "./remoteStream";
import { quotedPrompt } from "./replyQuote";

/**
 * `?norepo=1` makes the demo workspace a plain directory, so the History tab's
 * absence is something the harness captures rather than merely claims.
 */
const noRepo = new URLSearchParams(window.location.search).get("norepo") === "1";

// The console output doubles as the "what is missing" list while iterating on
// the harness, which is why this one log call is intentional.
// eslint-disable-next-line no-console
const log = (...args: unknown[]) => console.log("[mock]", ...args);

let fusionSettings = initialFusionSettings();
const settingsModels = () => new URLSearchParams(window.location.search).has("fusionSettings") ? [...models, ...fusionSettingsModels.filter(model => !models.some(entry => entry.id === model.id && entry.provider === model.provider))] : models;
function saveFusionSettings(patch: FusionSettingsPatch) {
  fusionSettings = applyFusionSettings(fusionSettings, patch);
  void emit("app_settings_changed");
  return fusionSettings;
}
let settings: typeof appSettings = { ...appSettings };

/**
 * Threads the harness has put into a *running* state.
 *
 * A plain page load has nothing running (`list_streaming_thread_ids` is empty),
 * so the rail's reserved indicator slot could only ever show its placeholder:
 * the spinning ring, the waiting-approval badge and the completed/failed dots
 * were all unreachable in a capture. The seeding helpers below are the entry
 * points the harness calls from `shot/main.tsx` — harness-only, never part of
 * the fixture's default state.
 */
let harnessStreamingThreadIds: string[] = [];
/** Thread ids `?pinned=` added to the fixture's own pin (see `HarnessState`). */
let harnessPinnedThreadIds: string[] = [];
let harnessUpdate = "";

export interface HarnessState {
  /** Update screen fixtures, including recovery after the main UI stops. */
  update?: string | null;
  /** Thread ids the agent reports as streaming (drives the row's running ring). */
  streaming?: string[];
  /**
   * Thread ids whose finished run is unread (drives the completed/failed dot).
   *
   * Written straight into `sessionStorage` because that is where the app keeps
   * the set, and it is read at mount — seeding it from a React effect would be
   * too late. Each id also needs a terminal run in `runs`, since the dot is
   * gated on the run's own status.
   */
  unread?: string[];
  /**
   * Thread ids to pin for this capture (drives the Vibe panel's membership).
   *
   * The fixture ships one pinned conversation, which is what the rail's own
   * pinned section needs to be reachable. A panel with a single cell would say
   * nothing about the grid, so a capture that opens the panel pins a few more —
   * here, and only for that page load.
   */
  pinned?: string[];
}

export function seedHarnessState(state: HarnessState) {
  harnessUpdate = state.update ?? "";
  // Recorded, not applied: `threadList` is built below this function, so the flag
  // is stamped on when the list is read (see `list_threads`). Stamping is
  // idempotent, which is what makes that safe to do per read.
  harnessStreamingThreadIds = state.streaming ?? [];
  harnessPinnedThreadIds = state.pinned ?? [];
  if (state.unread) {
    try {
      sessionStorage.setItem("future.unreadThreads", JSON.stringify(state.unread));
    }
    catch {
      // Best-effort, as in the app's own hook.
    }
  }
}

/**
 * The keys the fixture's host has hidden.
 *
 * Read from the same mutable `settings` the mock answers `get_desktop_settings`
 * with, so a scenario that flips a switch and one that lists models cannot
 * disagree about what is hidden.
 */
function hiddenModelKeys(): string[] {
  const hidden: unknown = (settings as { hiddenModels?: unknown }).hiddenModels;
  return Array.isArray(hidden) ? hidden.filter((item): item is string => typeof item === "string") : [];
}

/**
 * Whether this page asked for a host whose *reads* take a moment (`?remoteSlowRead=1`).
 *
 * A host answers in a few milliseconds here, so the state a reader is in *while*
 * a read is in flight never reaches a capture — and that is exactly the state a
 * control with nothing to show lives in: one that filled its own label into the
 * value slot looked identical to one that said it was loading. This flag is the
 * fixture for that moment; nothing else about the host changes.
 */
function slowReadsEnabled(): boolean {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("remoteSlowRead") === "1";
}

/** One of the host's reads, held back to the fixture's `?remoteSlowRead=1`. */
function slow<T>(answer: T): T | Promise<T> {
  if (!slowReadsEnabled())
    return answer;
  return new Promise(resolve => setTimeout(resolve, 1500, answer));
}
const threadList = threads.map(thread => ({ ...thread }));
const workspaceList = workspaces.map(workspace => ({ ...workspace }));
/** Staged session-history pages, read back in 64 KiB chunks (see `begin_session_history_transfer`). */
const historyTransfers = new Map<string, Uint8Array>();

/**
 * The run a capture starts from the local composer: the journal it writes, and
 * the pushes that tell the window to read it.
 *
 * Every other local send in this harness ended the moment `agent_prompt`
 * answered, so there was no fixture for "a reply is being written" — and the
 * states a reader is in *while* one is (the composer's send-while-running
 * control, the send queue it fills) only exist during such a run. This one writes
 * frames on a timer and resolves when the last has landed, which is exactly what
 * the backend's stream lane does for a paired host (see `remoteStream.ts`).
 */
interface ShotRunEvent {
  id: string;
  runId: string;
  eventType: string;
  payload: string;
  sequence: number;
  createdAt: number;
}

const shotRunEvents = new Map<string, ShotRunEvent[]>();
let shotRunSeq = 0;
let shotEventSeq = 0;
let shotRuntimeRevision = 0;

/**
 * Whether this page's run calls a tool before it answers (`?toolRun=1`).
 *
 * Every other run here is thinking plus prose, so a settled reply's folded
 * "process" row could only ever be captured carrying a thought count. This flag
 * runs one completed `read` call instead of the reasoning, which is the row a
 * reader of a real reply opens to see which tools produced it. It changes
 * nothing unless asked for.
 */
function toolRunEnabled(): boolean {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("toolRun") === "1";
}

/** One completed `read` call: start then result, so the row settles. */
const TOOL_RUN_FRAMES: Array<[string, Record<string, unknown>]> = [
  ["tool_start", { tool_id: "shot_tool_1", tool_name: "read", tool_args: { path: "src/parser.ts" } }],
  ["tool_end", { tool_id: "shot_tool_1", tool_name: "read", is_error: false }],
];

/**
 * How long a scripted run should take, when the page asked for a slow one.
 *
 * The default run writes itself out in about three seconds, which is shorter
 * than a person needs to type a second message — so the send-while-running
 * control and the queue behind it were unreachable in a capture. `?slowStream=`
 * stretches the same frames across that many milliseconds (nothing else about
 * the run changes); absent, the pacing is exactly what it was.
 */
function slowStreamMs(): number {
  const raw = typeof window === "undefined" ? 0 : Number(new URLSearchParams(window.location.search).get("slowStream") ?? 0);
  return Number.isFinite(raw) && raw > 0 ? raw : 0;
}

/** One message waiting behind a running reply, as the Agent would hold it. */
interface ShotQueuedRun {
  runId: string;
  text: string;
  acceptedAt: string;
  state: string;
  /** The agent session the send named, so its run is announced on the same one. */
  sessionId: string;
}

/**
 * The conversation's Agent-side send queue, and the run it is waiting for.
 *
 * A prompt sent while a reply streams is *accepted and held*, not run: the
 * composer's queue row is a view of this list (`session_queue`), and the row
 * only leaves it when the run ahead of it ends. Without these two maps the
 * fixture could answer the queue read with nothing forever, which is why the
 * desktop send queue had no case — a queued message could not be produced, let
 * alone seen leaving.
 */
const shotQueue = new Map<string, ShotQueuedRun[]>();
const shotActiveRun = new Map<string, string>();

/**
 * How many waiting rows `?queue=<n>` seats in whichever conversation is read.
 *
 * The panel's own ceiling is the Agent's (`DEFAULT_SESSION_QUEUE_CAPACITY`, 128),
 * and a queue that long is what pushes the panel's first rows out of the frame:
 * the state a capture has to reach without pressing Send 128 times. Seeded on
 * the first read of a conversation and never re-seeded, so a row the reader
 * takes back stays taken back.
 */
function queuedFixtureSize(): number {
  return Number(new URLSearchParams(window.location.search).get("queue") ?? "") || 0;
}
const seededQueues = new Set<string>();

function seedQueuedFixture(threadId: string): void {
  const size = queuedFixtureSize();
  if (!threadId || size <= 0 || seededQueues.has(threadId))
    return;
  seededQueues.add(threadId);
  const now = Date.now();
  shotQueue.set(threadId, Array.from({ length: size }, (_, index) => ({
    runId: `shot_queued_${index + 1}`,
    // Three lines of the shape these rows arrive in: the panel clamps a row to
    // three lines, so this is the tallest a row can get.
    text: "[future-goal] goal goal_4ce46c920eb8: 1 event(s). Reconcile current status before acting; events may be stale. Read the goal's own state before answering.",
    acceptedAt: new Date(now - (size - index) * 20_000).toISOString(),
    state: "queued",
    // Nothing here starts one of these runs, so no session is ever announced on.
    sessionId: "",
  })));
}
/**
 * What this page has actually said, so a reload shows it.
 *
 * The fixture's transcripts are derived from the thread title and never
 * contained a sent message: any reload replaced the conversation with a
 * synthetic exchange and silently dropped what the reader had just written.
 * Kept per thread, appended in order, and used instead of the synthetic
 * transcript once a thread has one.
 */
const sentEntries = new Map<string, MockEntry[]>();

/**
 * The demo's pending cards the reader has answered, by approval id.
 *
 * The demo has two: the review session's escalation and the parser session's
 * push. `decide_approval_request` used to answer `null` and leave both in the
 * queue forever, so whichever conversation was on screen kept an authorisation
 * card above the composer for the whole session — through later turns, a new
 * message and the reply to it — because nothing could ever settle it. That is
 * the "stale approval card" a capture reports (#1533); the product settles it
 * (a run's terminal transition cancels its pending approvals,
 * `host/src/store/runs.rs::cancel_children_of_runs`), so the demo has to as
 * well, or it shows a state the shipped app cannot be in.
 */
const decidedApprovals = new Map<string, { status: string; decisionNote: string | null; decidedAt: number }>();

/** The two demo cards differ in shape (one suggests a saved rule, one does not). */
type DemoApprovalCard = typeof pendingApproval | typeof parserApproval;

/** The demo approvals still waiting on the reader. */
function demoApprovalCards(): DemoApprovalCard[] {
  const cards: DemoApprovalCard[] = [pendingApproval, parserApproval];
  return cards.filter(card => !decidedApprovals.has(card.id));
}

/** What this fixture's reply says, split into the frames a run writes. */
function shotRunFrames(prompt: string): Array<[string, Record<string, unknown>]> {
  return [
    ["agent_start", { started_at_ms: Date.now() }],
    ...(toolRunEnabled()
      ? TOOL_RUN_FRAMES
      : [
          ["thinking_start", {}],
          ["thinking_delta", { text: "先看这一轮要回答什么：" }],
          ["thinking_delta", { text: "把读者的问题读一遍，再决定动不动文件。" }],
          ["thinking_end", {}],
        ] as Array<[string, Record<string, unknown>]>),
    ["text_chunk", { text: "好，我先把" }],
    ["text_chunk", { text: `「${quotedPrompt(prompt)}」` }],
    ["text_chunk", { text: "拆成两步：" }],
    ["text_chunk", { text: "先核对一遍现有结果，" }],
    ["text_chunk", { text: "再按你说的改。" }],
  ];
}

function appendShotRunEvent(runId: string, eventType: string, data: Record<string, unknown>): void {
  const list = shotRunEvents.get(runId) ?? [];
  list.push({
    id: `shot_event_${++shotEventSeq}`,
    runId,
    eventType,
    payload: JSON.stringify(data),
    sequence: list.length,
    createdAt: Date.now(),
  });
  shotRunEvents.set(runId, list);
}

function publishShotRun(threadId: string, runId: string, status: string, resetProjection = false): void {
  void emit("thread-runtime-updated", {
    updates: [{ threadId, runId, revision: ++shotRuntimeRevision, status, resetProjection }],
  });
}

/**
 * Answer a local prompt as a streamed run: frames on a timer, then the reply.
 *
 * A stop ends it where it stands — the run's row is what says so (`abort_run`
 * marks it cancelled), so each tick checks it before writing again and hands back
 * the text produced so far. That is what the backend does for a real abort, and
 * it is what makes the app paint a stopped reply rather than a finished one.
 *
 * A prompt that arrives while one of this conversation's runs is still
 * streaming is not run at all: it is held in the conversation's queue and
 * answered with the Agent's `run_queued` ack, which is exactly what the app
 * branches on to leave the transcript alone and let the queue row carry the
 * words until the run ahead of it ends.
 */
async function streamShotRun(args: any): Promise<Record<string, unknown>> {
  const request = args?.request ?? {};
  const runId = String(request.runId ?? "");
  const threadId = String(request.threadId ?? "");
  const sessionId = String(request.sessionId ?? `sess_shot_${Date.now()}`);
  if (!runId || !threadId)
    return { content: "", complete: true, sessionId };

  const active = shotActiveRun.get(threadId);
  if (active && active !== runId) {
    const held = shotQueue.get(threadId) ?? [];
    held.push({ runId, text: String(request.message ?? ""), acceptedAt: new Date().toISOString(), state: "queued", sessionId });
    shotQueue.set(threadId, held);
    // `complete: false` on its own would read as a cut-off reply; the queued
    // kind is what says "accepted, still coming".
    return { content: "", complete: false, terminationKind: "run_queued", queuePosition: held.length, sessionId };
  }

  // The session the client named is what the queued run below has to be
  // announced on: a `user_message` event is only applied when its `sessionId`
  // matches the thread's own agent session (`useSessionMessageEvents`).
  shotActiveRun.set(threadId, runId);
  const startedAt = Date.now();
  const cancelled = () =>
    String((runs.find(candidate => candidate.id === runId) as { status?: string } | undefined)?.status ?? "") === "cancelled";
  const frames = shotRunFrames(String(request.message ?? ""));
  const slow = slowStreamMs();
  const frameDelay = slow ? Math.max(120, Math.round(slow / frames.length)) : 380;
  let written = "";
  for (const [eventType, data] of frames) {
    await new Promise(resolve => setTimeout(resolve, frameDelay));
    if (cancelled())
      break;
    appendShotRunEvent(runId, eventType, data);
    if (eventType === "text_chunk")
      written += String(data.text ?? "");
    publishShotRun(threadId, runId, "running");
  }
  if (cancelled()) {
    shotActiveRun.delete(threadId);
    publishShotRun(threadId, runId, "cancelled");
    void drainShotQueue(threadId);
    return { content: written, complete: false, sessionId };
  }
  await new Promise(resolve => setTimeout(resolve, frameDelay));
  publishShotRun(threadId, runId, "running");
  appendShotRunEvent(runId, "agent_end", {});
  recordSentTurn(threadId, runId, String(request.message ?? ""), written, Date.now() - startedAt);
  shotActiveRun.delete(threadId);
  // The Agent folds a burst of queued messages into the turn boundary: whatever
  // was held behind this reply starts now, in the order it was accepted.
  void drainShotQueue(threadId);
  return { content: written, complete: true, sessionId };
}

/** Keep what was said, so a history reload shows the conversation (see `sentEntries`). */
function recordSentTurn(threadId: string, runId: string, prompt: string, reply: string, durationMs: number): void {
  if (!prompt)
    return;
  const created = Date.now();
  const list = sentEntries.get(threadId) ?? [];
  // `runId` is what the transcript reconciliation keys on
  // (`historyMessageKey`): without it the reloaded pair is a different message
  // from the one on screen and the reader sees every sent prompt twice.
  list.push({
    id: `shot_u_${runId}`,
    kind: "message",
    role: "user",
    runId,
    createdAtMs: created,
    blocks: [{ kind: "text", text: prompt }],
    metadata: {},
  } as unknown as MockEntry);
  if (reply) {
    list.push({
      id: `shot_a_${runId}`,
      kind: "message",
      role: "assistant",
      runId,
      createdAtMs: created + 1,
      run: { status: "completed", durationMs },
      blocks: [{ kind: "text", text: reply }],
      metadata: {},
    } as unknown as MockEntry);
  }
  sentEntries.set(threadId, list);
}

/**
 * Start the next held message as if the Agent had reached the turn boundary.
 *
 * A queued prompt runs only after the reply it waited behind ends, and it is
 * announced the way the Agent announces it: a `user_message` agent event puts
 * the reader's own words in the transcript (the send that queued it deliberately
 * draws nothing), then the run streams its frames. Nothing in the app has to be
 * told about the queue — it reads `session_queue` and this run like any other.
 */
async function drainShotQueue(threadId: string): Promise<void> {
  const held = shotQueue.get(threadId) ?? [];
  const next = held.shift();
  if (!next)
    return;
  if (held.length === 0)
    shotQueue.delete(threadId);
  const run = runs.find(candidate => candidate.id === next.runId) as Record<string, unknown> | undefined;
  if (run)
    Object.assign(run, { status: "running", startedAt: Date.now(), endedAt: null, updatedAt: Date.now() });
  shotRunEvents.set(next.runId, []);
  shotActiveRun.set(threadId, next.runId);
  const startedAt = Date.now();
  publishShotRun(threadId, next.runId, "running", true);
  void emit("agent-event", {
    threadId,
    sessionId: next.sessionId,
    _eventType: "user_message",
    entry_id: `shot_entry_${next.runId}`,
    run_id: next.runId,
    text: next.text,
    created_at_ms: Date.now(),
  });
  const frames = shotRunFrames(next.text);
  const slow = slowStreamMs();
  const frameDelay = slow ? Math.max(120, Math.round(slow / frames.length)) : 380;
  let written = "";
  for (const [eventType, data] of frames) {
    await new Promise(resolve => setTimeout(resolve, frameDelay));
    appendShotRunEvent(next.runId, eventType, data);
    if (eventType === "text_chunk")
      written += String(data.text ?? "");
    publishShotRun(threadId, next.runId, "running");
  }
  await new Promise(resolve => setTimeout(resolve, frameDelay));
  appendShotRunEvent(next.runId, "agent_end", {});
  recordSentTurn(threadId, next.runId, next.text, written, Date.now() - startedAt);
  if (run)
    Object.assign(run, { status: "completed", endedAt: Date.now(), updatedAt: Date.now() });
  shotActiveRun.delete(threadId);
  publishShotRun(threadId, next.runId, "completed");
  void drainShotQueue(threadId);
}

let historyTransferSeq = 0;
/** The chunk size `historyTransfer.ts` insists on. */
const HISTORY_CHUNK_BYTES = 64 * 1024;

/** Directory names a checkout ignores; the demo tree never lists them. */
const IGNORED_TREE_NAMES = new Set([
  ".git",
  ".next",
  ".nuxt",
  ".turbo",
  ".cache",
  "build",
  "coverage",
  "dist",
  "node_modules",
  "out",
  "target",
]);
/** How many times each thread's agent state has been read (see the handler). */
const stateReads = new Map<string, number>();

/** Seed app settings before the app boots (`?settings=` in `main.tsx`). */
export function patchSettings(patch: Record<string, unknown>) {
  settings = { ...settings, ...patch };
  return settings;
}

export function entriesForThread(threadId: string): MockEntry[] {
  if (new URLSearchParams(window.location.search).has("fusion"))
    return fusionFixture.entries as unknown as MockEntry[];
  // `?compactHistory=1` swaps in the history of a run that compacted mid-turn,
  // so a capture can assert the reply after the divider survives the durable
  // projection (the schemaVersion 3 path).
  if (
    typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("compactHistory") === "1"
  ) {
    return compactResumeEntries;
  }
  return entriesByThread[threadId] ?? [];
}

/**
 * The conversation's own history once something has been said in it.
 *
 * A thread this page has sent a prompt in is not a demo transcript any more: it
 * is the exchange the reader just had. Returning it (and nothing else) is what
 * makes a reload — or the settle-reload after a re-attached run — show the
 * messages that were sent instead of replacing them with a synthetic pair.
 */
function threadEntries(threadId: string): MockEntry[] {
  const said = sentEntries.get(threadId);
  if (said && said.length > 0)
    return said;
  return entriesForThread(threadId);
}

/**
 * Demo files that exist as real PNGs under `shot/assets/`; a path maps to the
 * URL the browser can load.
 */
export function assetUrl(path: string): string {
  const known: Record<string, string> = {
    "effect-size.png": "/shot/assets/effect-size.png",
    "forest-plot.png": "/shot/assets/forest-plot.png",
    "parser-throughput.png": "/shot/assets/parser-throughput.png",
  };
  return known[path.split("/").pop() ?? ""] ?? path;
}

const EMPTY_REMOTE_STATUS = {
  agentAvailable: true,
  phase: "stopped",
  reason: null,
  recovery: null,
  natsUrl: "tls://www.future-os.cn:4222",
  pairId: "",
  pairingCode: null,
  pairingCodeExpiresAt: null,
  desktopId: "desktop_C76HV2R4TKPGCC55",
  desktopPublicKey: "MCowBQYDK2VwAyEA...",
  webUrl: null,
  webLanUrl: null,
  warningCode: null,
};

/**
 * The host side with several paired devices (`?remoteDevices=<mode>`).
 *
 * A plain load answers `EMPTY_REMOTE_STATUS`, which is the single-device page
 * the host section used to be. The device list only renders when the backend
 * reports `devices`, so a capture of it needs a fixture that has them — and the
 * three rows it can show are exactly the three states the list distinguishes:
 * the device in control, one connected but standing by (it stays connected so
 * it can take back over without re-pairing), and one that is only paired.
 */
function remoteStatusWithDevices(mode: string): Record<string, unknown> {
  const now = Math.floor(Date.now() / 1000);
  const devices = [
    {
      pairId: "pair_8F3A21C4D5E64B77",
      phase: "ready",
      control: "active",
      connected: true,
      client: { name: "iPhone 17 Pro", kind: "mobile" },
      pairingCode: null,
      pairingCodeExpiresAt: null,
      desktopId: "desktop_C76HV2R4TKPGCC55",
    },
    {
      pairId: "pair_2D9E4B1A7C3F8052",
      phase: "ready",
      control: "standby",
      connected: true,
      client: { name: "实验室的 MacBook Air", kind: "desktop" },
      pairingCode: null,
      pairingCodeExpiresAt: null,
      desktopId: "desktop_C76HV2R4TKPGCC55",
    },
    {
      pairId: "pair_6B7C0E5A9D21438F",
      phase: "stopped",
      control: "standby",
      connected: false,
      client: { name: "iPad mini", kind: "mobile" },
      pairingCode: null,
      pairingCodeExpiresAt: null,
      desktopId: "desktop_C76HV2R4TKPGCC55",
    },
  ];

  if (mode === "reconnect") {
    // The bridge has lost its connection and is retrying: the same device list,
    // with the transport down. `reason: "network"` is what turns the status row
    // into "网络异常" plus a "重新连接" action.
    return {
      ...EMPTY_REMOTE_STATUS,
      phase: "reconnecting",
      reason: "network",
      pairId: devices[0]!.pairId,
      devices: devices.map(device => ({ ...device, connected: false, control: "standby" })),
    };
  }

  if (mode === "waiting") {
    // The invitation for a fourth device is on screen: the list carries the
    // pending row, and the QR block below it is drawn from that row.
    const pending = {
      pairId: "pair_C41F7A08E62B4D19",
      phase: "connecting" as const,
      control: "standby" as const,
      connected: false,
      client: null,
      pairingCode: "futureos://remote/pair?code=mock&desktopId=desktop_C76HV2R4TKPGCC55",
      pairingCodeExpiresAt: now + 540,
      desktopId: "desktop_C76HV2R4TKPGCC55",
    };
    return {
      ...EMPTY_REMOTE_STATUS,
      phase: "ready",
      pairId: devices[0]!.pairId,
      devices: [...devices, pending],
      pairingCode: pending.pairingCode,
      pairingCodeExpiresAt: pending.pairingCodeExpiresAt,
    };
  }

  return {
    ...EMPTY_REMOTE_STATUS,
    phase: "ready",
    pairId: devices[0]!.pairId,
    devices,
  };
}

/** `?remoteDevices=multi|waiting|reconnect`, or `""` for the plain host page. */
function remoteDevicesMode(): string {
  if (typeof window === "undefined")
    return "";
  return new URLSearchParams(window.location.search).get("remoteDevices") ?? "";
}

/**
 * The paired computers this machine connects *out* to, with the states a screen
 * has to draw.
 *
 * `?peers=held` makes the first one a machine another paired device is driving:
 * its row says who holds it and refuses Settings until somebody takes it over.
 * The plain list is two peers — one reachable, one not — because "a host that
 * cannot be reached" is the state the merged list has to survive.
 */
function peerList(): typeof remotePeers {
  if (typeof window === "undefined")
    return remotePeers;
  if (new URLSearchParams(window.location.search).get("peers") !== "held")
    return remotePeers;
  return remotePeers.map((peer, index) => index === 0
    ? {
        ...peer,
        control: "standby" as const,
        holder: { pairId: "pair_phone", name: "LNA-AL00", kind: "mobile" },
      }
    : { ...peer, control: "active" as const });
}

/** Command name → implementation. Anything missing goes to the fallback. */
/** Mutable copy so a capture can show a task being toggled or deleted. */
const taskList = tasks.map(task => ({ ...task }));

const handlers: Record<string, (args: any) => unknown> = {
  get_fusion_settings: () => fusionSettings,
  update_fusion_settings: args => saveFusionSettings(args.input as FusionSettingsPatch),
  get_fusion: (args) => {
    const discussion = fusionProgressFixture(new URLSearchParams(window.location.search).get("fusion"));
    if (!args?.stepId) {
      const delay = Number(new URLSearchParams(window.location.search).get("fusionMetadataDelay") ?? 0);
      return delay > 0 ? new Promise(resolve => setTimeout(resolve, delay, discussion)) : discussion;
    }
    const step = discussion.steps?.find(s => s.id === args.stepId);
    if (!step)
      throw new Error("Unknown discussion step");
    const reviewLong = new URLSearchParams(window.location.search).has("fusionReviewLong") && step.phase === "reviewing";
    const publicOutput = reviewLong
      ? JSON.stringify({ participant: step.participant, current_answer: `# 当前答案\n\n依据已有报告，先确认现有存储约束，再确定方案。\n\n${"复核后的依据说明。\n\n".repeat(1500)}评议报告全文已读取。`, responses: [], self_review: [{ original_quote: "建议 SQLite", decision: "revise", updated_position: "先确认现有存储约束", reason: "以已有证据为准" }], mapping_corrections: [], additions: [] })
      : step.status === "completed" && step.phase === "issue_mapping"
        ? JSON.stringify(discussion.events?.find(event => event.kind === "issues")?.data)
        : step.status === "completed" && step.phase === "synthesizing" ? JSON.stringify(discussion.run?.result) : step.output;
    const output = new URLSearchParams(window.location.search).has("fusionLong") ? `${publicOutput}\n${"分页公开发言。".repeat(2400)}\n全文读取完成。` : publicOutput;
    const transcript = JSON.stringify([{ role: "user", content: [{ type: "text", text: step?.input }] }, { role: "assistant", content: [{ type: "text", text: output }] }]);
    const bytes = new TextEncoder().encode(transcript);
    const offset = Number(args.offset ?? 0);
    let end = Math.min(offset + 32768, bytes.length);
    while (end < bytes.length && (bytes[end]! & 0xC0) === 0x80)
      end--;
    const page = { ...step, transcript: new TextDecoder().decode(bytes.slice(offset, end)), next_offset: end, total_bytes: bytes.length };
    const delay = Number(new URLSearchParams(window.location.search).get("fusionReadDelay") ?? 0);
    return delay > 0 ? new Promise(resolve => setTimeout(resolve, delay, page)) : page;
  },
  // ── App bootstrap ──────────────────────────────────────────────────────
  initialize_app_store: () => null,
  app_build_info: () => buildInfo,
  get_app_settings: () => settings,
  update_app_settings: args => patchSettings(args?.input ?? {}),
  get_future_environment: () => ({ environment: "production", platformUrl: "https://api.future-os.cn" }),
  set_future_environment: () => ({ environment: "production", platformUrl: "https://api.future-os.cn" }),
  probe_sandbox: () => ({ available: true, tier: "sandbox", platform: "macos" }),
  // The desktop gates all account/provider work behind this handshake; without
  // it the app never leaves its splash ("请稍候").
  get_agent_status: () => ({
    phase: harnessUpdate.startsWith("stopped-") ? "stopping" : "ready",
    ...(harnessUpdate.startsWith("stopped-") ? { shutdown: { operationId: "shutdown-raw-id", reason: "update", state: "stopped", stage: "complete", elapsedMs: 500, errors: [] } } : {}),
    desktopVersion: buildInfo.version,
    agentVersion: buildInfo.version,
  }),
  // Seeded by the harness (`?streaming=`), so a row's *running* state — the
  // reserved slot's spinning ring — is reachable in a capture. Empty by
  // default: a plain page load has nothing running.
  list_streaming_thread_ids: () => harnessStreamingThreadIds,
  check_app_update: () => ({ currentVersion: buildInfo.version, latestVersion: "2.1.0", hasUpdate: Boolean(harnessUpdate), platformSupported: true, canInstallInApp: true, downloadUrl: "https://future-os.cn/" }),
  app_maintenance_status: () => ({
    cli: { tool: { path: "C:\\Program Files\\FutureOS\\future.exe", source: "bundled", version: buildInfo.version }, error: "CLI diagnostic must stay private" },
    update: !harnessUpdate || harnessUpdate === "available"
      ? null
      : {
          operationId: "update-raw-id",
          fromVersion: buildInfo.version,
          targetVersion: "2.1.0",
          stage: "finished",
          outcome: harnessUpdate.includes("repair") ? "repair-required" : harnessUpdate.includes("ready") ? "pending-validation" : "failed-restored",
          diagnostic: "Cannot start update observer: os error 50; C:\\Program Files\\FutureOS",
          restartSafe: !harnessUpdate.includes("repair"),
          repairAcknowledged: false,
        },
  }),
  get_skill_guide: () => ({
    links: { help: "https://future-os.cn/docs" },
    skills: {
      coachPrompt: { zh: "帮我把今天的文献整理成综述初稿", en: "Draft a review from today's papers" },
      manual: { zh: "https://future-os.cn/docs/skills", en: "https://future-os.cn/docs/skills" },
    },
  }),

  // ── Workspaces & threads ───────────────────────────────────────────────
  list_workspaces: () => workspaceList,
  list_workspaces_page: (args) => {
    const archived = Boolean(args?.archived);
    const all = archived
      ? []
      : workspaceList.filter(workspace => workspace.pinned !== true);
    return { workspaces: all.slice(0, Number(args?.limit ?? 50)), nextCursor: null };
  },
  list_pinned_workspaces: () => workspaceList.filter(workspace => workspace.pinned === true),
  pin_workspace: (args) => {
    const target = workspaceList.find(workspace => workspace.id === args?.input?.workspaceId);
    if (target)
      target.pinned = args.input.pinned;
    return target ?? null;
  },
  rename_workspace: (args) => {
    const target = workspaceList.find(workspace => workspace.id === args?.input?.workspaceId);
    if (target)
      target.name = args.input.name;
    return target ?? null;
  },
  delete_workspace: args => workspaceList.find(workspace => workspace.id === args?.workspaceId) ?? null,
  create_workspace: args => ({
    id: `ws_${Date.now()}`,
    name: args?.input?.name ?? "新工作区",
    kind: "user",
    path: args?.input?.path ?? `${HOME}/Research/new`,
    cleanupStatus: "active",
    createdAt: Date.now(),
    updatedAt: Date.now(),
  }),
  ensure_workspace_git: () => true,
  list_threads: () => {
    for (const id of harnessPinnedThreadIds) {
      const target = threadList.find(thread => thread.id === id);
      if (target)
        target.pinned = true;
    }
    return threadList;
  },
  list_threads_page: (args) => {
    const scope = String(args?.scope ?? "recent");
    const all = threadList.filter((thread) => {
      if (thread.status === "archived")
        return false;
      if (scope === "chat")
        return thread.mode === "chat";
      if (scope.startsWith("workspace:"))
        return thread.workspaceId === scope.slice("workspace:".length);
      return true;
    });
    const limit = Number(args?.limit ?? 50);
    return { threads: all.slice(0, limit), nextCursor: null, totalRoots: all.length };
  },
  list_pinned_threads: (args) => {
    const scope = String(args?.scope ?? "recent");
    return threadList.filter(thread => thread.pinned && (scope !== "chat" || thread.mode === "chat"));
  },
  list_active_threads: () => threadList.filter(thread => runs.some(run => run.threadId === thread.id && (run.status === "running" || run.status === "waiting_approval"))),
  get_recent_thread: () => threadList.find(thread => thread.id === (requestedThread() ?? "th_review")) ?? null,
  mark_thread_opened: () => null,
  pin_thread: (args) => {
    const target = threadList.find(thread => thread.id === args?.input?.threadId);
    if (target)
      target.pinned = args.input.pinned;
    return target ?? null;
  },
  rename_thread: (args) => {
    const target = threadList.find(thread => thread.id === args?.input?.threadId);
    if (target)
      target.title = args.input.title;
    return target ?? null;
  },
  generate_thread_title: () => ({ title: "多巴胺与风险决策：任务不确定性下的效应方向", model: "deepseek-v4" }),
  update_thread_model: args => threadList.find(thread => thread.id === args?.input?.threadId) ?? null,
  update_thread_thinking_level: args => threadList.find(thread => thread.id === args?.input?.threadId) ?? null,
  delete_thread: () => null,
  restore_thread: () => null,
  batch_delete_threads: () => ({ deletedCount: 0, failed: [] }),
  get_thread_cleanup_summary: args => ({
    threadId: args?.threadId ?? "",
    workspaceId: "ws_dopamine",
    workspaceKind: "user",
    workspacePath: `${HOME}/Research/dopamine-decision`,
    cleanupStatus: "active",
    artifactCount: 3,
    workspaceFileCount: 12,
  }),
  fork_thread: () => `sess_fork_${Date.now()}`,
  observe_session: () => null,
  get_thread_agent_state: (args) => {
    const target = threadList.find(thread => thread.id === args?.threadId) ?? threadList[0];
    if (!target)
      return null;
    // The first read for a thread answers with the state the header already
    // shows; later reads (the app re-reads when the usage panel opens) answer
    // with a larger figure, so a capture can prove the panel refreshed.
    const reads = (stateReads.get(target.id) ?? 0) + 1;
    stateReads.set(target.id, reads);
    const chatMode = target.mode === "chat";
    const usage = chatMode
      ? unpricedSessionUsage
      : reads > 1
        ? refreshedSessionUsage
        : sessionUsage;
    return {
      model: target.model ?? "future/deepseek-v4-pro",
      thinkingLevel: "medium",
      sessionName: target.title,
      sessionId: target.agentSessionId ?? null,
      cwd: workspaceList.find(workspace => workspace.id === target.workspaceId)?.path ?? `${HOME}/Research/dopamine-decision`,
      parentSessionId: target.parentSessionId ?? null,
      isStreaming: false,
      isCompacting: false,
      activeRun: null,
      // Chat-mode conversations stand in for a model with no prices on file, so
      // the usage dialog's tokens-only fallback is capturable.
      usage,
    };
  },
  reconcile_thread_workspace: () => null,
  attach_remote_stream: () => ({ runId: null }),

  // ── Session history ────────────────────────────────────────────────────
  /**
   * A history read, staged and then read back in chunks.
   *
   * This is the lane `integrations/storage/historyTransfer.ts` uses: the backend
   * stages the page, the client reads 64 KiB slices of it and composes the JSON
   * cooperatively. Only the inline command was answered here, so every
   * conversation in every capture came out as `invalid_history_transfer` and a
   * retry link — the transcript a screenshot exists to show never rendered at
   * all. The bytes are UTF-8 JSON, which is what `decodeJsonBytes` expects.
   */
  begin_session_history_transfer: (args) => {
    const entries = threadEntries(args?.threadId ?? "");
    const page = { entries, hasMore: false, nextOffset: entries.length };
    const bytes = new TextEncoder().encode(JSON.stringify(page));
    const id = `shot_transfer_${++historyTransferSeq}`;
    historyTransfers.set(id, bytes);
    return { id, totalBytes: bytes.length, chunkBytes: HISTORY_CHUNK_BYTES };
  },
  read_session_history_transfer: (args) => {
    const bytes = historyTransfers.get(String(args?.transferId ?? ""));
    if (!bytes)
      return [];
    const offset = Math.max(0, Number(args?.offset ?? 0));
    return Array.from(bytes.subarray(offset, offset + HISTORY_CHUNK_BYTES));
  },
  release_session_history_transfer: (args) => {
    historyTransfers.delete(String(args?.transferId ?? ""));
    return null;
  },
  get_session_entries_page: (args) => {
    const entries = threadEntries(args?.threadId ?? "");
    return { entries, hasMore: false, nextOffset: entries.length };
  },
  get_session_entries: args => ({ entries: threadEntries(args?.threadId ?? "") }),

  // ── Runs, tools, approvals ─────────────────────────────────────────────
  list_runs: args => runs.filter(run => run.threadId === args?.threadId),
  get_latest_run: args => runs.find(run => run.threadId === args?.threadId) ?? null,
  get_run: args => runs.find(run => run.id === args?.runId) ?? null,
  list_latest_run_infos: (args: any) =>
    (args?.threadIds ?? [])
      .map((threadId: string) => {
        const run = runs.find(candidate => candidate.threadId === threadId);
        return run ? { threadId, runId: run.id, status: run.status, endedAt: run.endedAt ?? null } : null;
      })
      .filter(Boolean),
  create_run: (args) => {
    const run = {
      id: `run_shot_${++shotRunSeq}`,
      threadId: String(args?.input?.threadId ?? ""),
      triggerMessageId: args?.input?.triggerMessageId ?? null,
      status: "running",
      modelProvider: args?.input?.modelProvider ?? "future",
      modelId: args?.input?.modelId ?? "deepseek-v4-pro",
      startedAt: Date.now(),
      endedAt: null,
      errorMessage: null,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };
    runs.unshift(run as never);
    shotRunEvents.set(run.id, []);
    publishShotRun(run.threadId, run.id, "running", true);
    return run;
  },
  update_run_status: (args) => {
    const run = runs.find(candidate => candidate.id === args?.input?.runId);
    if (!run)
      return null;
    const status = String(args?.input?.status ?? run.status);
    Object.assign(run as unknown as Record<string, unknown>, {
      status,
      errorMessage: args?.input?.errorMessage ?? null,
      endedAt: status === "running" ? (run.endedAt ?? null) : Date.now(),
      updatedAt: Date.now(),
    });
    publishShotRun(run.threadId, run.id, status);
    return run;
  },
  /**
   * A stop the backend honours on its own clock.
   *
   * The row turns `cancelled` a moment later, not inside the call: a real abort
   * has to reach the running turn, be acked and settle the row, and the state a
   * reader is in during that gap — reply still on screen, queue still waiting — is
   * exactly what a capture of "interrupting" has to show.
   */
  abort_run: (args) => {
    const run = runs.find(candidate => candidate.id === args?.runId);
    if (!run)
      return null;
    setTimeout(() => {
      Object.assign(run as unknown as Record<string, unknown>, { status: "cancelled", endedAt: Date.now(), updatedAt: Date.now() });
      publishShotRun(run.threadId, run.id, "cancelled");
    }, 900);
    return run;
  },
  archive_finished_runs: () => 2,
  list_run_events: (args: any) => shotRunEvents.get(String(args?.runId ?? "")) ?? [],
  list_run_events_since: (args: any) => {
    const events = shotRunEvents.get(String(args?.runId ?? "")) ?? [];
    const since = Number(args?.sinceSequence ?? -1);
    return since < 0 ? events : events.filter(event => event.sequence > since);
  },
  list_run_events_bulk: (args: any) => (args?.runIds ?? []).map((id: string) => [id, shotRunEvents.get(id) ?? []]),
  list_tool_calls: args => toolCalls.filter(call => call.runId === args?.runId),
  list_tool_calls_bulk: (args: any) =>
    (args?.runIds ?? []).map((id: string) => [id, new URLSearchParams(window.location.search).has("fusion")
      ? id === "run_1" ? [{ id: "fusion-tool", runId: id, name: "fusion", kind: "fusion", input: JSON.stringify(fusionFixture.discussion.run.input), status: "completed", createdAt: 1793260000000 }] : []
      : toolCalls.filter(call => call.runId === id)]),
  list_tool_outputs: (args) => {
    const content = toolOutputs[args?.toolCallId];
    return content
      ? [{ id: `out_${args.toolCallId}`, toolCallId: args.toolCallId, kind: "text", content, createdAt: Date.now() }]
      : [];
  },
  /**
   * The cards waiting on the reader, per conversation.
   *
   * `th_review` has the fixture's original escalation (the forest plot); the
   * parser session has its own — the push the two commits are waiting for (see
   * `parserApproval`). Both are `sandbox_escalation`, so the card localizes its
   * own title instead of showing the agent's English string.
   */
  list_approval_requests: (args) => {
    if (new URLSearchParams(window.location.search).has("fusion"))
      return [];
    const threadId = args?.threadId;
    return demoApprovalCards().filter(card => card.threadId === threadId);
  },
  /**
   * A run's auto-approval assessments: none here, but the *handler* has to exist.
   *
   * The run inspector renders them unconditionally, so an unanswered command
   * reaches it as `null` and `data.filter` throws — which unmounts the whole app.
   * Opening a step row (or the panel) is how a capture reaches that screen, so a
   * missing answer here cost every frame that clicked a tool row, not a detail.
   */
  list_approval_assessments: () => [],
  list_pending_approval_requests: () => new URLSearchParams(window.location.search).has("fusion") ? [] : demoApprovalCards(),

  /**
   * Record the reader's answer, the way the backend does.
   *
   * The card leaves the queue (and the sidebar's pending badge with it) and the
   * decision is kept, so a later read of the conversation does not bring it
   * back. A decision that names an unknown card is answered with `null` instead
   * of inventing one.
   */
  decide_approval_request: (args) => {
    const input = args?.input ?? {};
    const id = String(input.approvalRequestId ?? "");
    const status = String(input.status ?? "");
    const card = demoApprovalCards().find(entry => entry.id === id);
    if (!card || !["approved", "rejected", "cancelled"].includes(status))
      return null;
    const decision = { status, decisionNote: input.decisionNote ?? null, decidedAt: Date.now() };
    decidedApprovals.set(id, decision);
    return { ...card, ...decision };
  },
  save_approval_rule: () => null,
  save_approval_rules: () => null,

  // ── Review ─────────────────────────────────────────────────────────────
  // `isGitWorkspace` is what the review panel gates its history tab on, so the
  // `norepo` flag has to answer here as well as in `get_git_history`: a demo
  // workspace that is a plain directory is one in every read, not just one.
  get_workspace_review_capabilities: () => ({
    isGitWorkspace: !noRepo,
    views: noRepo ? ["last_run"] : ["git_changes", "last_run"],
    defaultView: noRepo ? "last_run" : "git_changes",
    changePreview: "ready",
  }),
  get_git_review: () => ({
    isGitWorkspace: !noRepo,
    workspacePath: `${HOME}/Research/dopamine-decision`,
    branch: "main",
    upstream: "origin/main",
    diffBase: null,
    diffBaseLabel: "工作区未提交的改动",
    additions: 18,
    deletions: 2,
    files: reviewFiles.map(file => ({
      path: file.path,
      status: file.changeType,
      additions: file.additions,
      deletions: file.deletions,
      diff: file.diff,
      binary: file.path.endsWith(".png"),
      diffTruncated: false,
    })),
  }),
  /**
   * The demo checkout's history, in the shape the phone's sheet reads too.
   *
   * The gutters are the host's own layout output, not hand-drawn: the decoder
   * reads glyphs, so an invented grid would decode into a picture nobody asked
   * for.
   */
  get_git_history: () => ({
    isRepo: !noRepo,
    repoName: noRepo ? "" : "dopamine-decision",
    path: noRepo ? `${HOME}/Research/notes` : `${HOME}/Research/dopamine-decision`,
    commits: noRepo ? [] : gitHistoryCommits,
    truncated: false,
  }),
  get_git_commit: () => ({
    short: "9f2c1a7",
    patch: gitHistoryPatch,
    truncated: false,
  }),
  get_last_run_review: () => ({
    changeset: {
      id: "cs1",
      threadId: "th_review",
      runId: "run_1",
      title: "本次运行的改动",
      status: "applied",
      filesChanged: 2,
      additions: 18,
      deletions: 2,
      sourceKind: "run_snapshot",
      workspaceId: "ws_dopamine",
      binaryFiles: 1,
      omittedFiles: 0,
      completeness: "complete",
      confidence: "normal",
      overlapped: false,
      createdAt: Date.now(),
      updatedAt: Date.now(),
    },
    files: reviewFiles,
    run: runs[0],
    snapshotStatus: "complete",
    confidence: "normal",
    overlapped: false,
  }),
  retry_run_review: () => null,

  // ── Files & artifacts ──────────────────────────────────────────────────
  /**
   * A directory of the demo checkout.
   *
   * Build output is dropped here rather than kept out of `workspaceFiles` by
   * hand: `node_modules`, `target`, `.git` and the rest are what a real tree
   * hides behind its ignore list, and a product screenshot that lists them reads
   * as an untidy workspace. Filtering in the handler means a fixture entry added
   * later cannot leak one back into the panel.
   */
  list_directory: (args) => {
    const entries = workspaceFiles[args?.path] ?? [];
    return entries.filter(entry => !IGNORED_TREE_NAMES.has(entry.name));
  },
  search_workspace_files: () => [],
  // `validUtf8` and `size` are part of the command's real contract: without
  // `validUtf8` the text preview classifies every file as binary and bails to
  // the OS handler. Demo files also preview as themselves — a `.py` shows the
  // source the reply quotes, not the demo markdown — so the code preview's
  // syntax highlighting has real input.
  read_text_file_preview: (args) => {
    const name = (args?.path ?? "").split("/").pop() ?? "";
    const content = demoFileContents[name] ?? "# 多巴胺与风险决策：结论对比\n\n见下方表格。\n";
    return {
      path: args?.path ?? "",
      name,
      content,
      size: content.length,
      truncated: false,
      validUtf8: true,
    };
  },
  open_path: () => null,
  open_external_url: () => null,
  open_url: () => null,
  resolve_preview_link_path: args => ({ path: args?.target ?? "", name: "file.md" }),
  resolve_markdown_references: (args) => {
    const references = args?.input?.references ?? [];
    return references.map((reference: any) => ({
      targetId: reference.targetId,
      targetType: reference.targetType,
      status: reference.targetType === "file" ? "resolved" : "missing",
      data: reference.targetType === "file"
        ? {
            path: reference.targetId,
            name: String(reference.targetId).split("/").pop() ?? "file",
            relativePath: String(reference.targetId).replace(`${HOME}/Research/dopamine-decision/`, ""),
            insideWorkspace: String(reference.targetId).startsWith(`${HOME}/Research/dopamine-decision/`),
          }
        : null,
    }));
  },
  list_artifacts: () => [],
  delete_artifact: () => null,
  export_artifact_file: () => null,
  import_attachment_artifact: () => null,
  import_ephemeral_attachment: () => null,
  inspect_attachment: args => ({ path: args?.path, name: "attachment", kind: "file", size: 1024 }),
  validate_image_attachment: () => ({ valid: true }),
  save_pasted_file: () => null,
  save_pasted_image: () => null,
  delete_temp_attachment: () => null,
  read_native_clipboard_file_paths: () => [],
  generate_image_thumbnail: () => null,
  prepare_image_preview: args => ({ path: assetUrl(args?.path ?? ""), version: "1" }),

  // ── Agent: models, providers, account ──────────────────────────────────
  list_agent_models: () => settingsModels(),
  /**
   * A local prompt, answered as a streamed run (see `streamShotRun`).
   *
   * The composer's send-while-running control and the send queue it fills exist
   * only while this is in flight, so a capture of either needs a reply that
   * takes seconds and writes itself out — the same lane a paired host's stream
   * feeds (`remoteStream.ts`), on this machine's own conversation.
   */
  agent_prompt: async (args: any) => streamShotRun(args),
  /**
   * The conversation's Agent-side queue, as the composer's queue row reads it.
   *
   * `queued` is what a second send while a reply streams fills (see
   * `streamShotRun`), and it empties on its own when the run ahead of it ends.
   * `recentTerminalAcks` stays empty: nothing here drops a row without running
   * it, so there is never a sentence to explain.
   */
  session_queue: (args: any) => {
    const threadId = String(args?.threadId ?? "");
    seedQueuedFixture(threadId);
    const queued = (shotQueue.get(threadId) ?? []).map((row, index) => ({
      runId: row.runId,
      position: index + 1,
      acceptedAt: row.acceptedAt,
      text: row.text,
      state: row.state,
    }));
    return { activeRunId: shotActiveRun.get(threadId) ?? null, queued, recentTerminalAcks: [] };
  },
  /** Taking a waiting message back out before it runs (`取消队列`) . */
  cancel_queued_run: (args: any) => {
    const threadId = String(args?.threadId ?? "");
    const runId = String(args?.runId ?? "");
    const held = (shotQueue.get(threadId) ?? []).filter(row => row.runId !== runId);
    if (held.length === 0)
      shotQueue.delete(threadId);
    else
      shotQueue.set(threadId, held);
    return null;
  },
  sync_future_models: () => ({ synced: true, modelCount: models.length, revision: 42 }),
  list_agent_providers: () => providersView,
  upsert_custom_provider: () => providersView,
  update_builtin_provider: () => providersView,
  update_builtin_provider_key: () => providersView,
  set_builtin_provider_base_url: () => providersView,
  delete_custom_provider: () => providersView,
  // Thinking-level detection: every level is accepted, so a screenshot scenario
  // that clicks Detect shows a fully selected row.
  probe_thinking_level: (args: { input?: { level?: string } }) => ({
    level: args?.input?.level || "default",
    ok: true,
  }),
  set_default_model: () => null,
  get_future_auth_state: () => ({
    status: "authenticated",
    profile: { email: "lixin@lab.example.edu", userId: "u_8841", emailVerified: true, createdAt: "2026-03-02T08:00:00Z" },
  }),
  get_future_profile: () => ({
    email: "lixin@lab.example.edu",
    userId: "u_8841",
    emailVerified: true,
    createdAt: "2026-03-02T08:00:00Z",
  }),
  get_future_balance: () => ({ credits: 842.5 }),
  start_future_login: () => ({ loginUrl: "https://future-os.cn/login", expiresAt: Date.now() + 300_000 }),
  poll_future_login: () => ({ status: "pending" }),
  logout_future_provider: () => null,

  // ── Skills ─────────────────────────────────────────────────────────────
  list_installed_skills: () => installedSkills,
  list_available_skills: () => availableSkills,
  install_skill: () => null,
  uninstall_skill: () => true,
  refresh_skills: () => null,
  bootstrap_builtin_skills: () => null,

  // ── Skill recommendation (PRD "技能推荐") ─────────────────────────────
  // The agent answers `suggest_skill`; the daily-budget state lives in the
  // Desktop store. Both are simulated here so a scenario can render the
  // recommendation card without a Jev key. `suggest_skill` returns the first
  // uninstalled catalogue skill so the card has content to show.
  suggest_skill: () => {
    const installedIds = new Set((installedSkills as { id: string }[]).map(row => row.id));
    const pick = (availableSkills as { id: string; description: string }[])
      .find(entry => !installedIds.has(entry.id));
    return pick ? { name: pick.id, description: pick.description } : null;
  },
  skill_reco_today: () => ({ count: 0, skillIds: [], messageHashes: [] }),
  record_skill_reco: () => null,

  // ── Remote (phone control) ─────────────────────────────────────────────
  // A plain load is the single-device host page; `?remoteDevices=` swaps in the
  // multi-device fixture (see `remoteStatusWithDevices`).
  remote_status: () =>
    remoteDevicesMode() ? remoteStatusWithDevices(remoteDevicesMode()) : EMPTY_REMOTE_STATUS,
  remote_pairing_status: () => ({ paired: false, deviceName: null }),
  remote_start: () => ({
    ...EMPTY_REMOTE_STATUS,
    phase: "ready",
    pairId: "E59C28514F3F453CB4E91949930396F5",
    pairingCode: "eyJkZXNrdG9wIjoiZGVza3RvcF9DNzZIVjJSNFRLUEdDQzU1In0",
    pairingCodeExpiresAt: Math.floor(Date.now() / 1000) + 600,
  }),
  remote_stop: () => EMPTY_REMOTE_STATUS,
  remote_unpair: () => null,
  // The three actions the device list offers. Serving them keeps a capture that
  // presses one from silently doing nothing while it works in the app.
  remote_add_device: () => remoteStatusWithDevices("waiting"),
  remote_takeover: () => remoteStatusWithDevices("multi"),
  remote_unpair_device: () => remoteStatusWithDevices("multi"),

  // ── Remote, the client role: this Desktop connecting *out* ─────────────
  //
  // Everything here answers the opposite question to the host handlers above:
  // what this machine is connected to, not what is connected to it. The two
  // halves are separate screens reading separate commands, so a fixture that
  // served both from one shape would hide a screen reading the wrong one.
  remote_peer_list: () => peerList(),
  // A freshly claimed host has no name yet and is not connected until the
  // supervisor gets there — which is the state the pairing screen shows.
  remote_peer_pair: () => ({
    ...remotePeers[0]!,
    desktopId: "desktop_paired",
    name: null,
    pairId: "pair_paired",
    connected: false,
    error: null,
    bridgeInstanceId: null,
    features: [],
    agentAvailable: false,
  }),
  remote_peer_connect: (args: { desktopId: string }) => ({
    ...(remotePeers.find(peer => peer.desktopId === args.desktopId) ?? remotePeers[0]),
    connected: true,
    error: null,
  }),
  /**
   * The app's own liveness pass, and "reconnect all".
   *
   * The fixture answers it with the same list: a capture's state *is* the
   * fixture, and the list below is static, so a machine the fixture calls down
   * stays down on screen however often the app asks for it. Serving it at all is
   * what keeps a screen that clicks "reconnect all" from silently doing nothing
   * in a capture while it works in the app.
   */
  remote_peer_connect_all: () => remotePeers,
  remote_peer_disconnect: () => null,
  remote_peer_unpair: () => null,
  remote_peer_set_label: () => null,
  /**
   * Stage a file on the host.
   *
   * Absent until now, and its absence did not degrade gracefully: `attach()`
   * awaited `null` and then pushed it into `uploads`, so the very next render
   * read `upload.uploadId` off nothing and unmounted the whole page. Every
   * capture that tried to show a staged attachment on the remote side
   * therefore came out blank.
   */
  remote_peer_upload_file: (args: { path?: string; name?: string }) => {
    const path = args?.path ?? "attachment";
    const name = args?.name ?? path.split("/").pop() ?? path;
    return { uploadId: `upl_${name}`, name, contentHash: `hash_${name}` };
  },
  remote_peer_sessions: (args: { desktopId: string }) => ({
    desktopId: args.desktopId,
    sessions: remoteSessions[args.desktopId] ?? [],
  }),
  remote_peer_workspaces: (args: { desktopId: string }) => ({
    ...remoteWorkspaces,
    desktopId: args.desktopId,
  }),
  /**
   * That host's directory listing, and a preview of one file in it.
   *
   * The preview answers a *local cache* path whose name is the host's, which is
   * what the overlay is handed: the name is what selects a renderer, and the
   * asset map turns these two demo names into real PNGs under `shot/assets/`.
   */
  remote_peer_list_files: (args: { desktopId: string; sessionId: string; path?: string }) => ({
    ...remoteFileListing,
    path: args?.path ?? remoteFileListing.path,
  }),
  remote_peer_preview_file: (args: { name?: string }) => ({
    name: args?.name ?? "preview",
    path: `${HOME}/.future/app/remote-previews/mock__${args?.name ?? "preview"}`,
  }),
  /**
   * The command channel to a host.
   *
   * Routed by command type here exactly as the host routes it: a fixture that
   * answered everything with one blob would not reveal a screen asking for the
   * wrong command.
   */
  remote_peer_request: (args: { command?: { type?: string; [key: string]: unknown }; desktopId: string }) => {
    const command = args.command ?? {};
    if (command.type === "prompt") {
      // The host answers the prompt, then streams the run it started (a fixture
      // the `remote-streaming` capture drives; see `remoteStream.ts`).
      streamRemoteRun(String(command.sessionId ?? ""), String(command.message ?? ""));
      return { sessionId: String(command.sessionId ?? ""), threadId: "th_forest" };
    }
    switch (command.type) {
      case "get_fusion":
        return handlers.get_fusion!({ stepId: command.entryId, offset: command.offset });
      case "get_fusion_settings":
        return fusionSettings;
      case "update_fusion_settings":
        return saveFusionSettings(command.fusionSettings as FusionSettingsPatch);
      case "get_session_entries":
        return {
          entries: sessionEntries(String(command.sessionId)),
          hasMore: false,
          nextOffset: 0,
        };
      /**
       * The host's state: its settings *and* the run it has in flight.
       *
       * `activeRun` is what a client reads to learn there is a reply already being
       * written — the pushes alone cannot say so until a frame arrives, and a long
       * tool call pushes nothing for minutes.
       */
      case "get_state":
        return slow({ ...remoteConversationState, ...replayState(String(command.sessionId ?? "")) });
      /**
       * The run's log, for the client that reads it rather than guessing.
       *
       * Served for the fixture's in-flight run only; anything else answers an
       * empty read, which is what a host with nothing to add says.
       */
      case "get_events_since":
        return replayPage(
          String(command.sessionId ?? ""),
          String(command.runId ?? ""),
          Number(command.sinceIdx ?? -1),
        );
      /**
       * The host's own unit: one conversation, addressed by its *thread* id.
       *
       * Applied to the fixture, so the list stops serving the row and a capture
       * of the panel after a delete shows what the delete actually did. Leaving
       * it out made a deleted conversation stay in the list — the fixture
       * disagreeing with the command rather than the screen.
       */
      case "delete_session": {
        const threadId = String(command.threadId ?? "");
        const rows = remoteSessions[args.desktopId] as Array<{ threadId?: string }> | undefined;
        const index = rows?.findIndex(row => row.threadId === threadId) ?? -1;
        if (rows && index >= 0)
          rows.splice(index, 1);
        return { deleted: true };
      }
      // The host answers these two differently on purpose: `list_models` is the
      // picker's list and omits what the user hid, `list_settings_models` is the
      // settings page's and includes it. The fixture mirrors that split, or a
      // page fed from the wrong one would still look right here.
      case "list_models":
      case "get_available_models":
        return slow({
          // The host answers with the level the *agent* declares for a model,
          // which is a capability marker rather than a default anybody chose:
          // `high` for every reasoning model and `off` for the rest
          // (`agent/src/rpc/commands/providers.rs`). The fixture's own `models`
          // carry the level this app's pickers start at, so the host's answer is
          // mapped rather than reused: a client that seeded a draft from this
          // field would look right here and run at high effort against a real
          // machine (see `useRemoteConversationSettings`).
          models: models
            .filter(model => !hiddenModelKeys().includes(`${model.provider}/${model.id}`))
            .map(model => ({ ...model, thinkingLevel: model.reasoning === false ? "off" : "high" })),
        });
      case "list_settings_models":
        return { models: settingsModels() };
      case "get_desktop_settings":
        return { ...settings };
      case "update_desktop_settings": {
        const patch = (command.settings ?? {}) as Record<string, unknown>;
        settings = { ...settings, ...patch };
        return { ...settings };
      }
      case "get_settings":
        return { approvalTier: "sandbox", sandboxAvailable: true };
      case "list_workspaces":
        return remoteWorkspaces;
      case "get_tool_call_args":
        return remoteToolArgs[String(command.toolCallId)] ?? null;
      case "list_session_files":
        return { files: [] };
      case "list_skills":
        return { skills: installedSkills };
      case "list_available_skills":
        return { skills: availableSkills };
      case "list_tasks":
        return { tasks: tasks.map(task => ({
          id: task.id,
          name: task.name,
          enabled: task.enabled,
          triggerKind: task.triggerKind,
          trigger: task.trigger,
          depCount: task.depCount,
          nextDueAt: task.nextDueAt,
          queued: task.queued,
          latestRun: task.latestRun,
        })) };
      // The host's list row omits the prompt; its detail record is what carries
      // it, which is why the client completes each row with this read.
      case "get_task": {
        const task = tasks.find(row => row.id === String(command.taskId));
        return task ? { ...task } : null;
      }
      case "list_providers":
        return providersView;
      case "list_task_runs":
        return { runs: taskRuns[String(command.taskId)] ?? [] };
      case "list_task_deps":
        return { deps: taskDeps[String(command.taskId)] ?? [] };
      case "skill_reco_today":
        return { today: { count: 0, skillIds: [], messageHashes: [] } };
      case "suggest_skill":
        return { skill: null };
      case "generate_session_title":
        return { title: "重画森林图并核对置信区间（已修正区间水平）" };
      default:
        return null;
    }
  },

  // ── Embedded terminal ──────────────────────────────────────────────────
  // Points at `shot/terminal-server.mjs`, a tiny stand-in for the loopback PTY
  // server that replays a canned shell session.
  terminal_server_info: () => ({
    url: "http://127.0.0.1:7391",
    token: "shot-token",
    port: 7391,
    maxSessions: 8,
  }),

  // ── Tasks ──────────────────────────────────────────────────────────────
  // The panel reads through these; mutations are applied to the in-memory list
  // so a capture shows the same list a real write would produce.
  list_tasks: () => taskList,
  create_task: (args) => {
    const input = args?.input ?? {};
    const created = {
      id: `tsk_${Date.now()}`,
      name: input.name,
      enabled: input.enabled ?? true,
      prompt: input.prompt,
      promptVersion: 1,
      cwd: input.cwd,
      modelId: input.modelId ?? null,
      thinkingLevel: input.thinkingLevel ?? null,
      sessionPolicy: input.sessionPolicy ?? "new",
      sessionRetention: input.sessionRetention ?? "keep",
      conversationMode: input.conversationMode ?? "workspace",
      triggerKind: input.triggerKind ?? "manual",
      trigger: input.trigger ?? {},
      depJoin: input.depJoin ?? "all",
      nextDueAt: input.triggerKind === "schedule" ? Date.now() + 60 * 60_000 : null,
      queued: false,
      depCount: 0,
      latestRun: null,
    };
    taskList.push(created);
    return created;
  },
  update_task: (args) => {
    const target = taskList.find(task => task.id === args?.id);
    if (!target)
      throw new Error("task not found");
    const input = args?.input ?? {};
    const promptChanged = input.prompt !== undefined && input.prompt !== target.prompt;
    Object.assign(target, {
      name: input.name ?? target.name,
      prompt: input.prompt ?? target.prompt,
      cwd: input.cwd ?? target.cwd,
      modelId: input.modelId ?? null,
      thinkingLevel: input.thinkingLevel ?? null,
      sessionPolicy: input.sessionPolicy ?? target.sessionPolicy,
      sessionRetention: input.sessionRetention ?? target.sessionRetention,
      triggerKind: input.triggerKind ?? target.triggerKind,
      trigger: input.trigger ?? target.trigger,
      depJoin: input.depJoin ?? target.depJoin,
      enabled: input.enabled ?? target.enabled,
      promptVersion: promptChanged ? target.promptVersion + 1 : target.promptVersion,
      nextDueAt: (input.triggerKind ?? target.triggerKind) === "schedule" ? Date.now() + 60 * 60_000 : null,
    });
    return target;
  },
  delete_task: (args) => {
    const index = taskList.findIndex(task => task.id === args?.id);
    if (index >= 0)
      taskList.splice(index, 1);
    return null;
  },
  set_task_enabled: (args) => {
    const target = taskList.find(task => task.id === args?.id);
    if (!target)
      throw new Error("task not found");
    target.enabled = !!args?.enabled;
    if (target.enabled && target.triggerKind === "schedule")
      target.nextDueAt = Date.now() + 60 * 60_000;
    return target;
  },
  run_task_now: args => taskList.find(task => task.id === args?.id) ?? null,
  list_task_runs: args => taskRuns[args?.id] ?? [],
  list_task_deps: args => taskDeps[args?.id] ?? [],

  // ── Settings pages ─────────────────────────────────────────────────────
  clear_app_data: () => null,
  reset_windows_sandbox: () => 0,
  install_app_update: () => null,
  restart_after_app_update: () => null,
  compact_thread_context: () => ({ operationId: "op_1", accepted: true }),
};

export function dispatch(command: string, args: any): unknown {
  const handler = handlers[command];
  if (!handler) {
    log("UNHANDLED COMMAND", command, JSON.stringify(args ?? {}).slice(0, 400));
    return null;
  }
  const value = handler(args);
  log(command, JSON.stringify(args ?? {}).slice(0, 200));
  return value;
}
