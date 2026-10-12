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

/** Whether this page asked for the streaming fixture. */
export function streamingEnabled(): boolean {
  return typeof window !== "undefined"
    && new URLSearchParams(window.location.search).get("remoteStream") === "1";
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

/** How long one frame takes. Slow enough that a capture lands mid-reply. */
const STREAM_STEP_MS = 220;

export function streamRemoteRun(sessionId: string, prompt: string): void {
  if (!streamingEnabled() || sessionId !== SESSION_ID)
    return;
  finished = [];
  // The question first, as the host publishes it when it accepts a prompt: the
  // transcript then shows what was asked before the answer starts arriving.
  const frames: Frame[] = [{ type: "user_message", data: { text: prompt } }, ...FRAMES];
  frames.forEach((frame, index) => {
    setTimeout(() => {
      emit("remote-peer-event", {
        desktopId: DESKTOP_ID,
        kind: "event",
        payload: {
          schemaVersion: 2,
          sessionId: SESSION_ID,
          type: frame.type,
          data: typeof frame.data === "string" ? frame.data : JSON.stringify(frame.data ?? {}),
          runId: RUN_ID,
          idx: index,
          epoch: 1,
          eventId: `${SESSION_ID}:${RUN_ID}:${index}`,
          timestamp: new Date(Date.now()).toISOString(),
          sessionIdx: index,
          runSequence: index,
        },
      });
    }, STREAM_STEP_MS * (index + 1));
  });
  // The run settles, and the host's journal has the reply: the transcript's
  // catch-up read then serves it (see `streamedEntries`).
  setTimeout(() => {
    const answered = buildAnswer();
    finished = [
      entry("re_stream_q", "user", prompt),
      ...answered,
      entry("re_stream_a", "assistant", "置信区间确实画成了 90%，我把区间改成 95% 后重画，并核对了每个效应的数值。", RUN_ID),
    ];
    emit("remote-peer-event", {
      desktopId: DESKTOP_ID,
      kind: "event",
      payload: {
        schemaVersion: 2,
        sessionId: SESSION_ID,
        type: "agent_end",
        data: JSON.stringify({}),
        runId: RUN_ID,
        idx: frames.length,
        epoch: 1,
        eventId: `${SESSION_ID}:${RUN_ID}:end`,
        timestamp: new Date(Date.now()).toISOString(),
        sessionIdx: frames.length,
        runSequence: frames.length,
      },
    });
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
): unknown {
  return {
    id,
    kind: kind ?? "message",
    role,
    ...(runId ? { runId } : {}),
    createdAtMs: Date.now(),
    blocks: [{ kind: "text", text }],
  };
}
