import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { BackHandler } from "react-native";
import { ChatScreen } from "../ChatScreen";
import { ComposerDock } from "../components/ComposerDock";
import { QUEUE_POLL_MS, type SessionQueueRow, type SessionQueueSnapshot } from "../../../remote/sessionQueue";

/**
 * One tap, one payload: the press that changes a waiting message's answer, driven
 * through the real panel and the real sheet inside the real chat screen.
 *
 * `ChatScreen.test.ts` mocks `ComposerDock` down to a host component, which is the
 * right trade for the ~70 behaviours it covers there — but it means the press
 * itself is never performed. `ComposerDock.test.ts` performs the press, and stops
 * at the callback. Nothing until this file crossed both: a reader could not tell
 * whether the answer reached the session, or only this box's draft. So this file
 * mounts the whole screen **unmocked** and asserts what the host is told.
 *
 * The fixture here is deliberately the smallest set the screen needs to draw a
 * conversation with one waiting row; anything the screen reads that this path does
 * not touch is left out rather than copied from the big file's fixture.
 */

const mockRemote = {
  credentials: { expectedDesktopId: "desktop" },
  desktops: [{ desktopId: "desktop", pairId: "pair_1", name: "Lab MacBook Pro", icon: "laptop" }],
  selectedSessionId: "history",
  selectedTitle: "History",
  sessionUsage: null,
  closeConversation: jest.fn(),
  refreshSessionUsage: jest.fn(async () => {}),
  sessions: [],
  workspaces: [],
  models: [],
  modelId: undefined as string | undefined,
  decideApproval: jest.fn(async () => {}),
  capabilities: new Set<string>(),
  timeline: {
    items: [
      { id: "u-last", kind: "message", role: "user", text: "Question" },
      { id: "a-last", kind: "message", role: "assistant", text: "Short answer" },
    ],
  },
  canLoadOlderTimeline: false,
  loadingOlderTimeline: false,
  loadOlderTimeline: jest.fn(async () => false as const),
  reloadTimeline: jest.fn(),
  timelineError: null as string | null,
  retryTimeline: jest.fn(),
  selectSession: jest.fn(async () => {}),
  resolveToolCallTarget: jest.fn(async () => null),
  listSessionFiles: jest.fn(async () => null),
  listGitCommits: jest.fn(async () => ({ isRepo: false, repoName: "", path: "", commits: [], truncated: false })),
  getGitCommit: jest.fn(async () => ({ short: "", patch: "", truncated: false })),
  desktopOnline: true,
  streaming: false,
  timelineSyncStatus: "idle",
  connectionPresentation: { level: "connected" },
  presence: null,
  takeControl: jest.fn(async () => {}),
  sendMessage: jest.fn(async () => {}),
  getQueue: jest.fn(async (): Promise<SessionQueueSnapshot> => ({ acks: [], rows: [] })),
  cancelQueuedRun: jest.fn(async () => {}),
  releaseQueuedRun: jest.fn(async () => {}),
  editQueuedRun: jest.fn(async () => {}),
  removeQueuedAttachment: jest.fn(async () => {}),
  resendQueuedRun: jest.fn(async () => {}),
  archived: false,
  archiveSession: jest.fn(async () => {}),
  draft: false,
  draftWorkspaceId: undefined as string | undefined,
  busy: false,
  busyReason: undefined as string | undefined,
  refreshSessions: jest.fn(async () => {}),
  loadSkills: jest.fn(async () => []),
};

jest.mock("../../../remote/RemoteContext", () => ({
  useRemote: () => mockRemote,
  useRemoteControls: () => mockRemote,
}));

// The icon package ships ESM only, and every icon here is decoration: name each
// one after itself so a tree walk still finds a stable host component. A proxy
// rather than a hand-kept list, which would break the day the dock gains a
// chevron.
jest.mock("lucide-react-native", () => new Proxy({}, {
  get: (_target, name) => (name === "__esModule" ? true : String(name)),
}));
// The transcript rows are not what this file is about, and they pull in the whole
// markdown pipeline.
jest.mock("../messages/TimelineCard", () => ({ TimelineCard: "TimelineCard" }));
// Phone layout insets, which every surface here reads: zero at the edges is what a
// test wants anyway.
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ bottom: 0, left: 0, right: 0, top: 0 }),
}));

let tree: ReactTestRenderer;

beforeEach(() => {
  jest.clearAllMocks();
  jest.spyOn(BackHandler, "addEventListener").mockReturnValue({ remove: jest.fn() });
  jest.useFakeTimers();
  mockRemote.selectedSessionId = "history";
  mockRemote.streaming = false;
  mockRemote.getQueue.mockResolvedValue({ acks: [], rows: [] });
});

afterEach(() => {
  act(() => tree?.unmount());
  jest.useRealTimers();
});

const answer = {
  mode: "single" as const,
  options: [{ id: "forest", label: "重画森林图" }, { id: "flow", label: "补一张流程图" }],
  selectedIds: ["forest"],
  title: "接下来做哪一件？",
};

const row: SessionQueueRow = {
  acceptedAt: "2026-01-01T00:00:00Z",
  attachments: [],
  choice: answer,
  position: 1,
  runId: "run_a",
  state: "queued",
  text: "[接下来做哪一件？ → 重画森林图]",
};

const press = (testID: string) => {
  const node = tree.root.findAll(
    candidate => candidate.props.testID === testID && typeof candidate.props.onPress === "function",
  )[0];
  if (!node)
    throw new Error(`nothing pressable is drawn with testID ${testID}`);
  act(() => node.props.onPress());
};

test("pressing a waiting row's chip and picking an option edits that row on the session", async () => {
  mockRemote.getQueue.mockResolvedValue({ acks: [], rows: [row] });
  await act(async () => { tree = create(createElement(ChatScreen)); });
  // The queue is read for the conversation on screen; without that read there is
  // no row, and this test would pass by pressing nothing.
  await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
  expect(mockRemote.getQueue).toHaveBeenCalledWith("history");

  // The box this screen is writing into, read off the real dock: a queue row must
  // not fill it, because the next send would carry the same answer again.
  const draft = () => tree.root.findByType(ComposerDock).props.choice;
  expect(draft()).toBeNull();

  press("send-queue-toggle");
  press("send-queue-choice");
  press("choice-sheet-option-flow");

  expect(mockRemote.editQueuedRun).toHaveBeenCalledWith(
    "history",
    "run_a",
    "[接下来做哪一件？ → 补一张流程图]",
    {
      mode: "single",
      options: answer.options,
      selectedIds: ["flow"],
      title: "接下来做哪一件？",
    },
  );
  // The row was fixed in place: nothing was taken back, nothing re-sent, and the
  // pick never became this box's draft.
  expect(mockRemote.cancelQueuedRun).not.toHaveBeenCalled();
  expect(mockRemote.sendMessage).not.toHaveBeenCalled();
  expect(draft()).toBeNull();
});

test("unticking the last pick of a waiting row leaves the row exactly as it was", async () => {
  // Zero ticks is not an answer (§7): the row is a message that is already waiting,
  // so rewriting it to `[question → ]` would hand the agent a question it reads as
  // an answer. Two layers refuse it — the dock returns before handing the pick up,
  // and the screen’s own handler refuses with a toast behind that. This pins the
  // **outcome** (nothing reaches the host and the sheet stays up on the untouched
  // row), not which layer fired: removing either guard alone still leaves it green,
  // and removing both turns it red.
  const multi: SessionQueueRow = {
    ...row,
    choice: { ...answer, mode: "multi", selectedIds: ["forest"] },
    text: "[接下来做哪一件？ → 重画森林图]",
  };
  mockRemote.getQueue.mockResolvedValue({ acks: [], rows: [multi] });
  await act(async () => { tree = create(createElement(ChatScreen)); });
  await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });

  press("send-queue-toggle");
  press("send-queue-choice");
  // The row has exactly one tick, and this press takes it away.
  press("choice-sheet-option-forest");

  expect(mockRemote.editQueuedRun).not.toHaveBeenCalled();
  expect(mockRemote.sendMessage).not.toHaveBeenCalled();
  // The sheet stays up on the pick that cannot be made, rather than closing on a
  // change that never happened.
  expect(tree.root.findAll(
    candidate => candidate.props.testID === "choice-sheet-option-flow",
  ).length).toBeGreaterThan(0);
});
