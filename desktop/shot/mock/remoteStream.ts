import { remoteEntries } from "./data";
import { emit } from "./event";

/**
 * A remote run, streamed into the harness page.
 *
 * The client-role screens are rendered against a *host* that only answers when
 * asked: the mock's catalogue, transcripts and settings are all static, so a
 * capture of a remote conversation showed a transcript and nothing moving. The
 * run a reader actually watches — the one their own prompt started — is pushed,
 * and this is that push, in the host's own event vocabulary and its own body
 * shape (see `remote/publisher.rs::build_event_body`).
 *
 * Gated on `?remoteStream=1` so every other capture keeps the quiet host: a
 * scenario that opens a conversation and finds deltas arriving in it would be
 * measuring this fixture rather than the screen it is about.
 */

/** The demo conversation whose run is streamed; the one the scenario opens. */
const SESSION_ID = "rs_forest";
const DESKTOP_ID = "desktop_studio";
const RUN_ID = "run_stream_1";

/** How long one frame takes. Slow enough that a capture lands mid-reply. */
const STREAM_STEP_MS = 220;

/** Whether this page asked for the streaming fixture. */
export function streamingEnabled(): boolean {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("remoteStream") === "1";
}

/**
 * Whether this page is the mid-flight fixture: a conversation whose newest run was
 * already going when the window opened.
 *
 * Its own flag rather than a share of `?remoteStream=1`, because the two fixtures
 * disagree about the same conversation: that one opens a quiet conversation and
 * starts a run with its own prompt, and this one opens a conversation a run is
 * already inside. A page that asked for both would be describing two states at
 * once — and the state read below would claim a run is in flight while the other
 * fixture's prompt started a different one.
 */
export function midFlightEnabled(): boolean {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("remoteMidFlight") === "1";
}

/**
 * Whether the host should answer on the *lean* lane — the trimmed envelope the
 * real host sends once a client declares `lean_events_v1`.
 *
 * The other fixtures emit the full body, so nothing in the harness exercised the
 * trim: a client that had come to depend on a field the trim removes would look
 * perfectly healthy in every capture. This mode drops exactly what
 * `remote_host::lean` drops, so the screens a reader sees are the ones the lean
 * lane actually produces.
 */
export function leanEnabled(): boolean {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("remoteLean") === "1";
}

/**
 * The run the fixture's transcript is already part-way through.
 *
 * `rs_forest` ends with a turn whose run the fixture still reports as *running*:
 * the state a reader sees when they open a conversation while the host is
 * answering it. A capture of that conversation could show the copy the page
 * carried, but nothing after it — the rest of the reply only exists on the push
 * lane, and this is that continuation, under the run the page already names.
 */
const MID_FLIGHT_RUN_ID = "rrun_2";
const MID_FLIGHT_ENTRY_ID = "re_4";

/**
 * When that run started, as the run itself reported it.
 *
 * Eight minutes before the page loads, which is what makes the timer on this
 * reply an assertion rather than a coincidence: a client that only ever saw the
 * pushes counts its own arrival, and would show seconds.
 */
export const MID_FLIGHT_STARTED_MS = Date.now() - 8 * 60_000;

/**
 * The run's log, up to the moment the reader opens the conversation.
 *
 * The host's journal holds everything a run has written — including the part that
 * predates any window: the run's own start, the reasoning, the command it ran, and
 * the text of the reply so far. A window that joins here has watched none of it,
 * so this is the only place it exists for that window.
 *
 * The *record* (the history page) carries the same run as one entry with the first
 * model call's text — `re_4` in `data.ts` — which is why the reply the reader sees
 * without this log starts mid-thought.
 */
const MID_FLIGHT_LOG: Frame[] = [
  { type: "agent_start", data: { started_at_ms: MID_FLIGHT_STARTED_MS } },
  { type: "thinking_start" },
  { type: "thinking_delta", data: { text: "两张图各自硬编码了区间，不受影响；" } },
  { type: "thinking_delta", data: { text: "结论要落到文件里，不能只停在回复里。" } },
  { type: "thinking_end" },
  { type: "tool_start", data: { tool_id: "call_note", tool_name: "write", tool_args: { path: "analysis/forest-ci-note.md" } } },
  { type: "tool_end", data: { tool_id: "call_note", tool_name: "write", result: "written", exit_code: 0 } },
  { type: "text_chunk", data: { text: "另外两张图（`fig2-forest.R`、`figS3.R`）各自硬编码了区间，不受影响；只有这一处。" } },
];

/**
 * The frames the run writes *after* the reader is watching.
 *
 * Their indices continue the log's, which is how a real run behaves: the pushed
 * frames and the journal are one sequence, so a window that read the log and then
 * follows the pushes holds the whole reply exactly once.
 */
const MID_FLIGHT_FRAMES: Frame[] = [
  { type: "text_chunk", data: { text: "我把结论写进 `analysis/forest-ci-note.md`：" } },
  { type: "text_chunk", data: { text: "`ci_level` 是唯一要改的参数，" } },
  { type: "text_chunk", data: { text: "改完重跑 `Rscript analysis/effect-size.R` 就行。" } },
];

/** The frames of this run the host has published so far (its journal grows). */
let midFlightPublished = 0;

/**
 * The host's answer to `get_events_since` for the run in flight.
 *
 * The whole log for a read from its beginning, the events after `sinceIdx` for a
 * tail, and `rawEvents` throughout: this host trims its feed (the desktop declares
 * `lean_events_v1`), which is what tells a client that a hole between indices is
 * by design rather than frames it never got.
 */
export function replayPage(sessionId: string, runId: string, sinceIdx: number): unknown {
  if (!midFlightEnabled() || sessionId !== SESSION_ID || runId !== MID_FLIGHT_RUN_ID)
    return { events: [], watermark: -1, rawEvents: 0 };
  const log = [...MID_FLIGHT_LOG, ...MID_FLIGHT_FRAMES.slice(0, midFlightPublished)];
  const events = log.map((frame, index) => ({
    type: frame.type,
    data: typeof frame.data === "string" ? frame.data : JSON.stringify(frame.data ?? {}),
    runId,
    idx: index,
  }));
  const watermark = events.length - 1;
  const covered = events.filter(event => event.idx > sinceIdx);
  return { events: covered, watermark, rawEvents: covered.length };
}

/**
 * The reply the run ends with: what the page already carried, plus the tail.
 *
 * One string, because the host's journal holds one entry — its mid-run snapshot
 * and its finished form are the same record, which is what makes the settled
 * transcript draw the reply once.
 */
const MID_FLIGHT_REPLY = "另外两张图（`fig2-forest.R`、`figS3.R`）各自硬编码了区间，不受影响；只有这一处。"
  + "我把结论写进 `analysis/forest-ci-note.md`：`ci_level` 是唯一要改的参数，改完重跑 `Rscript analysis/effect-size.R` 就行。";

/** The finished reply, or null while the run is still being written. */
let midFlightReply: string | null = null;
/** Whether the mid-flight run has been started: one run per page. */
let midFlightStarted = false;

/**
 * What the host reports about the conversation: the run it has in flight.
 *
 * The state read is how a client learns there is something to read before a frame
 * happens to arrive — a long tool call pushes nothing for minutes — so the fixture
 * has to answer it for the mid-flight capture to mean anything.
 */
export function replayState(sessionId: string): { activeRun?: { runId: string } } {
  if (!midFlightEnabled() || sessionId !== SESSION_ID || midFlightReply !== null)
    return {};
  return { activeRun: { runId: MID_FLIGHT_RUN_ID } };
}

/**
 * Continue the run the transcript is already part-way through.
 *
 * Fired by a scenario step rather than a timer: what is being captured is what a
 * reader sees *after* opening a running conversation, so the frames must start
 * where the reader's own open put them.
 */
export function startMidFlightRun(sessionId: string): void {
  if (!midFlightEnabled() || sessionId !== SESSION_ID || midFlightStarted)
    return;
  midFlightStarted = true;
  const base = MID_FLIGHT_LOG.length;
  MID_FLIGHT_FRAMES.forEach((frame, index) => {
    setTimeout(() => {
      midFlightPublished = index + 1;
      publishFrame(MID_FLIGHT_RUN_ID, frame, base + index);
    }, STREAM_STEP_MS * (index + 1));
  });
  // The run ends and its journal holds the whole reply: the client's catch-up
  // read then serves it, under the entry the page already had.
  setTimeout(() => {
    midFlightReply = MID_FLIGHT_REPLY;
    publishFrame(
      MID_FLIGHT_RUN_ID,
      { type: "agent_end", data: { duration_ms: 8 * 60_000 } },
      base + MID_FLIGHT_FRAMES.length,
      `${SESSION_ID}:${MID_FLIGHT_RUN_ID}:end`,
    );
  }, STREAM_STEP_MS * (MID_FLIGHT_FRAMES.length + 6));
}

/**
 * The fixture's in-flight turn, as the finished reply: the same entry — the host's
 * journal holds one record per model call — carrying the whole reply, the
 * reasoning, and the call that wrote the note.
 */
function settledReply(entry: unknown, reply: string): unknown | null {
  const record = entry as { id?: string; blocks?: Array<{ kind?: string }> };
  if (record.id !== MID_FLIGHT_ENTRY_ID || !Array.isArray(record.blocks))
    return null;
  return {
    ...record,
    run: { status: "completed", durationMs: 26_000 },
    blocks: [
      ...record.blocks.map(block => (block.kind === "text" ? { ...block, text: reply } : block)),
      { kind: "reasoning", text: "结论得落在文件里，不能只停在回复里。" },
      {
        kind: "tool_call",
        toolCallId: "call_note",
        name: "write",
        arguments: { path: "analysis/forest-ci-note.md" },
      },
    ],
  };
}

/**
 * The reply the streamed run produced, as the host's journal would have it.
 *
 * Kept so the history read serves it after the run ends: the client re-reads the
 * transcript when a run settles, and a fixture whose read still returned the
 * stale entries would make the settled reply vanish — which is a bug the capture
 * would then be showing instead of the behaviour it is about.
 */
let finished: unknown[] = [];

export function streamedEntries(sessionId: string): unknown[] {
  return sessionId === SESSION_ID ? finished : [];
}

export function sessionEntries(sessionId: string): unknown[] {
  const fixture = remoteEntries[sessionId] ?? [];
  // The mid-flight run's own turn is the fixture's — the reply the agent had
  // already written to its journal when the window opened. Once the run settles,
  // the same entry carries the whole reply, as the host's journal does.
  const reply = midFlightReply;
  const transcript = sessionId === SESSION_ID && reply
    ? fixture.map(entry => settledReply(entry, reply) ?? entry)
    : fixture;
  return [...transcript, ...streamedEntries(sessionId)];
}

/** One frame of the run: its type and the JSON payload `data` carries. */
interface Frame {
  data?: unknown;
  type: string;
}

/**
 * The reply, frame by frame — a question answered with reasoning, a little text,
 * one command, and the answer that follows it.
 *
 * Written out rather than generated: this is what the transcript has to render
 * (a thinking block, an inline tool row that resolves, text on both sides of it),
 * and a loop over "some deltas" would not exercise the ordering.
 */
const FRAMES: Frame[] = [
  { type: "agent_start" },
  { type: "thinking_start" },
  { type: "thinking_delta", data: { text: "The plot is drawn from " } },
  { type: "thinking_delta", data: { text: "effect-size.R; read it first." } },
  { type: "thinking_end" },
  { type: "tool_start", data: { tool_id: "call_1", tool_name: "read", tool_args: { path: "effect-size.R" } } },
  { type: "tool_end", data: { tool_id: "call_1", tool_name: "read", result: "…", exit_code: 0 } },
  { type: "text_chunk", data: { text: "置信区间确实画成了 90%，" } },
  { type: "text_chunk", data: { text: "我把区间改成 95% 后重画，" } },
  { type: "text_chunk", data: { text: "并核对了每个效应的数值。" } },
];

/**
 * Start the run's stream, as the host would after accepting this prompt.
 *
 * The pacing is the point of the fixture: a capture has to catch the reply
 * mid-flight to show that it streams, so the frames arrive over a few seconds
 * rather than in one tick.
 */

/**
 * One frame, published in the host's own body shape.
 *
 * The lean envelope is the same one the real host's trim produces: `eventId`,
 * `timestamp`, `epoch`, `sessionIdx`, `runSequence` and `schemaVersion` are what
 * it removes, and `sessionId` is what it deliberately keeps (the `state.events`
 * mirror has no conversation in its subject).
 */
function publishFrame(runId: string, frame: Frame, index: number, eventId?: string): void {
  const data = typeof frame.data === "string" ? frame.data : JSON.stringify(frame.data ?? {});
  emit("remote-peer-event", {
    desktopId: DESKTOP_ID,
    kind: "event",
    payload: leanEnabled()
      ? { sessionId: SESSION_ID, type: frame.type, data, runId, idx: index }
      : {
          schemaVersion: 2,
          sessionId: SESSION_ID,
          type: frame.type,
          data,
          runId,
          idx: index,
          epoch: 1,
          eventId: eventId ?? `${SESSION_ID}:${runId}:${index}`,
          timestamp: new Date(Date.now()).toISOString(),
          sessionIdx: index,
          runSequence: index,
        },
  });
}

export function streamRemoteRun(sessionId: string, prompt: string): void {
  if (!streamingEnabled() || sessionId !== SESSION_ID)
    return;
  finished = [];
  // The question first, as the host publishes it when it accepts a prompt: the
  // transcript then shows what was asked before the answer starts arriving.
  const frames: Frame[] = [{ type: "user_message", data: { text: prompt } }, ...FRAMES];
  frames.forEach((frame, index) => {
    setTimeout(publishFrame, STREAM_STEP_MS * (index + 1), RUN_ID, frame, index);
  });
  // The run settles, and the host's journal has the reply: the transcript's
  // catch-up read then serves it (see `streamedEntries`). The settle signal is
  // trimmed like the frames above: it has to keep working on the lean lane, or a
  // lean client would watch a reply that never becomes a record.
  setTimeout(() => {
    const answered = buildAnswer();
    finished = [
      entry("re_stream_q", "user", prompt),
      ...answered,
      entry(
        "re_stream_a",
        "assistant",
        "置信区间确实画成了 90%，我把区间改成 95% 后重画，并核对了每个效应的数值。",
        RUN_ID,
        undefined,
        { status: "completed", durationMs: 6_000 },
      ),
    ];
    publishFrame(RUN_ID, { type: "agent_end", data: {} }, frames.length, `${SESSION_ID}:${RUN_ID}:end`);
  }, STREAM_STEP_MS * (frames.length + 6));
}

/**
 * The thinking and tool entries the settled transcript carries.
 *
 * A settled exchange is not just its text: the reasoning and the command that
 * produced the answer are entries of their own, and the projection folds them
 * back into the same row the live reply drew.
 */
function buildAnswer(): unknown[] {
  return [
    entry("re_stream_t", "assistant", "The plot is drawn from effect-size.R; read it first.", RUN_ID, "reasoning"),
    {
      id: "re_stream_tool",
      kind: "message",
      role: "assistant",
      runId: RUN_ID,
      createdAtMs: Date.now(),
      blocks: [{
        kind: "tool_call",
        toolCallId: "call_1",
        name: "read",
        arguments: { path: "effect-size.R" },
      }],
    },
  ];
}

function entry(
  id: string,
  role: "assistant" | "user",
  text: string,
  runId?: string,
  kind?: string,
  run?: { status: string; durationMs?: number },
): unknown {
  return {
    id,
    kind: kind ?? "message",
    role,
    ...(runId ? { runId } : {}),
    // A settled entry carries the run's outcome, which is what tells a client
    // reading the record back that this run is over.
    ...(run ? { run } : {}),
    createdAtMs: Date.now(),
    blocks: [{ kind: "text", text }],
  };
}
