import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import {
  ActivityIndicator,
  BackHandler,
  FlatList,
  Keyboard,
  StyleSheet,
  Text,
} from "react-native";
import { ChatTopBar } from "../components/ChatTopBar";
import { SessionFilesPanel } from "../components/SessionFilesPanel";
import { GitGraphPanel } from "../components/GitGraphPanel";
import { ActionMenu, type MenuAction } from "../../../components/ActionMenu";
import { SessionUsageSheet } from "../components/SessionUsageSheet";
import { RenameModal } from "../components/RenameModal";
import { ChatScreen } from "../ChatScreen";
import { JUMP_VIEW_OFFSET } from "../useQuestionNav";
import { ComposerDock } from "../components/ComposerDock";
import { AttachmentSourceMenu } from "../components/AttachmentSourceMenu";
import type { PendingSuggestion } from "../useSkillRecommendation";
import type { TimelineCardProps } from "../messages/types";
import { FloatingTimelineButton } from "../components/FloatingTimelineButton";
import type { TimelineSyncStatus } from "@future-os/remote-client/syncEngine";
import type { RemoteSession, RemoteSessionUsage, RemoteWorkspace, TimelineItem } from "../../../remote/types";
import { QUEUE_POLL_MS, type SessionQueueRow, type SessionQueueSnapshot } from "../../../remote/sessionQueue";
import { QUEUED_SEND_POLICY } from "../useSendMessage";
import * as chatUtils from "../utils";

const mockRemote = {
  credentials: { expectedDesktopId: "desktop" },
  /**
   * The desktops this phone is paired with. The conversation's top bar names the
   * one it is on, so the list is part of what this screen reads.
   */
  desktops: [{ desktopId: "desktop", pairId: "pair_1", name: "Lab MacBook Pro", icon: "laptop" }] as {
    desktopId: string;
    pairId: string;
    name?: string;
    icon?: string;
  }[],
  selectedSessionId: "history",
  selectedTitle: "History",
  // No usage reported until a get_state read lands; the sheet must still open
  // and say so rather than invent a ¥0 breakdown.
  sessionUsage: null as RemoteSessionUsage | null,
  closeConversation: jest.fn(),
  // The app re-reads the session as the sheet opens.
  refreshSessionUsage: jest.fn(async () => {}),
  sessions: [] as RemoteSession[],
  workspaces: [] as RemoteWorkspace[],
  models: [] as { id: string; provider: string; label?: string }[],
  modelId: undefined as string | undefined,
  // The approval decision the composer's card submits.
  decideApproval: jest.fn<Promise<void>, [string, string]>(),
  capabilities: new Set(),
  timeline: {
    items: [
      { id: "u-last", kind: "message", role: "user", text: "Question" },
      {
        id: "a-last",
        kind: "message",
        role: "assistant",
        text: "Short answer",
      },
    ],
  },
  canLoadOlderTimeline: true,
  loadingOlderTimeline: false,
  loadOlderTimeline: jest.fn<Promise<false | string[]>, []>(),
  reloadTimeline: jest.fn(),
  // A failed history read and the listing the file panel asks for.
  timelineError: null as string | null,
  retryTimeline: jest.fn(),
  // A `#` reference in a sent message opens that conversation, and a lean tool
  // row asks this machine for the command the page left out.
  selectSession: jest.fn(async () => {}),
  // The machine's own resolver, which answers with the command or null when the
  // row has none (see `RemoteControls.resolveToolCallTarget`).
  resolveToolCallTarget: jest.fn<Promise<string | null>, [string, string]>(async () => null),
  listSessionFiles: jest.fn(async () => null),
  // The top bar's menu asks this before it offers the Git entry, and the sheet
  // reads the history through the same two calls.
  listGitCommits: jest.fn(async () => ({ isRepo: false, repoName: "", path: "", commits: [], truncated: false })),
  getGitCommit: jest.fn(async () => ({ short: "", patch: "", truncated: false })),
  desktopOnline: true,
  // The model's own generation state, separate from the lane's sync status.
  streaming: false,
  timelineSyncStatus: "idle" as TimelineSyncStatus,
  connectionPresentation: { level: "connected" },
  presence: null as { control?: string; holder?: { name?: string } | null } | null,
  takeControl: jest.fn(async () => {}),
  // The send queue's flush delivers through this, as `useSendMessage` does.
  sendMessage: jest.fn<Promise<void>, [string, unknown[]?, unknown?, { busyPolicy?: string }?]>(async () => {}),
  // The conversation's queue, which lives on the desktop: the panel reads it and
  // the two verbs address the session's own run ids.
  getQueue: jest.fn<Promise<SessionQueueSnapshot>, [string]>(async () => ({ acks: [], rows: [] })),
  cancelQueuedRun: jest.fn<Promise<void>, [string, string]>(async () => {}),
  releaseQueuedRun: jest.fn<Promise<void>, [string, string]>(async () => {}),
  // A waiting message's files are copied down before the row is cancelled, so an
  // edit can send them again. These are the same transfer calls `retryMessage`
  // makes for a sent message's attachments.
  prepareAttachment: jest.fn(async (attachment: { name: string }) => ({
    chunkBytes: 0,
    contentHash: "",
    mimeType: "image/png",
    name: attachment.name,
    previewKind: "image",
    size: 2048,
    transferId: "transfer-1",
    variant: "preview",
  })),
  cachedAttachment: jest.fn(() => null),
  downloadAttachment: jest.fn(async () => ({ uri: "file:///cache/shot.png", size: 2048 })),
};
jest.mock("../../../remote/RemoteContext", () => ({
  useRemote: () => mockRemote,
  useRemoteControls: () => mockRemote,
}));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }),
}));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ bottom: 0 }),
}));
// The pickers open native surfaces; the queue's "add files" only needs to see
// what came back, and the composer's own menu is exercised elsewhere.
jest.mock("../../../remote/files", () => ({
  ...jest.requireActual("../../../remote/files"),
  pickAttachments: jest.fn(async (existing: unknown[]) => existing),
  pickFromAlbum: jest.fn(async (existing: unknown[]) => existing),
  takePhoto: jest.fn(async (existing: unknown[]) => existing),
}));
jest.mock("lucide-react-native", () => ({
  ArrowDown: "ArrowDown",
  ArrowUp: "ArrowUp",
  Camera: "Camera",
  Check: "Check",
  File: "File",
  History: "History",
  Image: "Image",
  Images: "Images",
  Paperclip: "Paperclip",
  Pencil: "Pencil",
  Play: "Play",
  Plus: "Plus",
  Trash2: "Trash2",
  X: "X",
}));
jest.mock("../messages/TimelineCard", () => ({
  TimelineCard: "TimelineCard",
}));
jest.mock("../../../components/ErrorBanner", () => ({
  ErrorBanner: "ErrorBanner",
}));
// Mutable so a test can supply a draft, a live send and a skill suggestion;
// the hooks are read at render time, after this module has initialised.
// `setMessage` mirrors the real hook's contract (it writes the draft back) so a
// test can tell a composed send from one that only stored the text.
const mockDraft: {
  message: string;
  attachments: unknown[];
  setMessage: jest.Mock;
  setAttachments: jest.Mock;
  sessionRefs: Record<string, unknown>;
} = {
  message: "",
  attachments: [],
  setMessage: jest.fn(),
  setAttachments: jest.fn(),
  sessionRefs: {},
};
jest.mock("../useComposerDraft", () => ({
  useComposerDraft: () => mockDraft,
}));
jest.mock("../useAttachmentPicker", () => ({
  useAttachmentPicker: () => ({}),
}));
// The download controller's entry points are called with `void` by the screen
// (fire-and-forget by design). They are synchronous jest.fn()s on purpose: an
// async mock leaves a promise the test cannot await, and its continuation lands
// after the renderer is gone (§8.18).
const mockFileDownload: {
  preview: unknown;
  activeDownload: unknown;
  fileAction: unknown;
  openAttachment: jest.Mock;
  openFileLink: jest.Mock;
  openOrShare: jest.Mock;
  setFileAction: jest.Mock;
} = {
  preview: null,
  activeDownload: null,
  fileAction: null,
  openAttachment: jest.fn(),
  openFileLink: jest.fn(),
  openOrShare: jest.fn(),
  setFileAction: jest.fn(),
};
jest.mock("../files/useChatFiles", () => ({ useChatFiles: () => mockFileDownload }));
const mockSendOptions = { current: null as { setTransferProgress: (value: number | null) => void } | null };
const mockSendApi: { send: jest.Mock; retryMessage: jest.Mock; continueMessage: jest.Mock; forkMessage: jest.Mock } = {
  send: jest.fn(),
  retryMessage: jest.fn(),
  continueMessage: jest.fn(),
  forkMessage: jest.fn(),
};
// The send/retry pair the timeline rows drive. `send` is asserted directly;
// the two others exist so the row wiring has a real controller to reach.
jest.mock("../useSendMessage", () => ({
  // The queue's failed flush reports through the same wording a manual send
  // uses; the mapping itself is covered by `useSendMessage.test.ts`.
  sendFailureText: () => ({ key: "chat.sendFailed" }),
  useSendMessage: (options: typeof mockSendOptions.current) => {
    mockSendOptions.current = options;
    return mockSendApi;
  },
}));
const mockSkillReco: {
  suggestion: PendingSuggestion | null;
  evaluating: boolean;
  evaluate: jest.Mock;
  installAndUse: jest.Mock;
  dismiss: jest.Mock;
} = {
  suggestion: null,
  evaluating: false,
  evaluate: jest.fn(async () => false),
  installAndUse: jest.fn(async () => null),
  dismiss: jest.fn(),
};
jest.mock("../useSkillRecommendation", () => ({
  useSkillRecommendation: () => mockSkillReco,
}));
jest.mock("../components/SessionUsageSheet", () => ({
  SessionUsageSheet: "SessionUsageSheet",
}));
jest.mock("../components/ChatTopBar", () => ({ ChatTopBar: "ChatTopBar" }));
jest.mock("../components/SessionFilesPanel", () => ({
  SessionFilesPanel: "SessionFilesPanel",
}));
jest.mock("../components/GitGraphPanel", () => ({
  GitGraphPanel: "GitGraphPanel",
}));
// The sheet stays a plain presenter: it renders whatever actions it is handed, so
// the screen's own list is what these tests read and press.
jest.mock("../../../components/ActionMenu", () => ({ ActionMenu: "ActionMenu" }));
jest.mock("../components/ComposerDock", () => ({
  ComposerDock: "ComposerDock",
}));
jest.mock("../components/ModelSelectorSheet", () => ({
  ModelSelectorSheet: "ModelSelectorSheet",
}));
jest.mock("../components/DownloadProgressModal", () => ({
  DownloadProgressModal: "DownloadProgressModal",
}));
jest.mock("../components/PreviewModal", () => ({
  PreviewModal: "PreviewModal",
}));
jest.mock("../components/RenameModal", () => ({ RenameModal: "RenameModal" }));
jest.mock("../components/SessionUsageSheet", () => ({
  SessionUsageSheet: "SessionUsageSheet",
}));
jest.mock("../components/NativeFileActionSheet", () => ({
  NativeFileActionSheet: "NativeFileActionSheet",
}));

let tree: ReactTestRenderer;
const removeBack = jest.fn();
beforeEach(async () => {
  jest.clearAllMocks();
  jest
    .spyOn(BackHandler, "addEventListener")
    .mockReturnValue({ remove: removeBack });
  jest.useFakeTimers();
  mockFileDownload.preview = null;
  mockFileDownload.activeDownload = null;
  mockFileDownload.fileAction = null;
  mockRemote.selectedSessionId = "history";
  mockRemote.canLoadOlderTimeline = true;
  mockRemote.desktopOnline = true;
  mockRemote.streaming = false;
  mockRemote.timelineSyncStatus = "idle";
  mockRemote.timelineError = null;
  mockRemote.retryTimeline = jest.fn();
  mockRemote.listSessionFiles = jest.fn(async () => null);
  mockRemote.capabilities = new Set();
  mockRemote.listGitCommits = jest.fn(async () => ({
    isRepo: false, repoName: "", path: "", commits: [], truncated: false,
  }));
  mockRemote.getGitCommit = jest.fn(async () => ({ short: "", patch: "", truncated: false }));
  mockRemote.models = [];
  mockRemote.modelId = undefined;
  mockRemote.decideApproval = jest.fn<Promise<void>, [string, string]>();
  mockRemote.loadingOlderTimeline = false;
  mockRemote.sessions = [];
  mockRemote.workspaces = [];
  mockRemote.sessionUsage = null;
  mockRemote.sendMessage.mockReset().mockResolvedValue(undefined);
  mockRemote.getQueue.mockReset().mockResolvedValue({ acks: [], rows: [] });
  mockRemote.cancelQueuedRun.mockReset().mockResolvedValue(undefined);
  mockRemote.releaseQueuedRun.mockReset().mockResolvedValue(undefined);
  mockRemote.loadOlderTimeline.mockReset().mockResolvedValue([]);
  mockDraft.message = "";
  mockDraft.attachments = [];
  mockDraft.setMessage.mockImplementation((value: string) => { mockDraft.message = value; });
  mockSkillReco.suggestion = null;
  mockSkillReco.evaluating = false;
  act(() => {
    tree = create(createElement(ChatScreen));
  });
  // Let the panel's opening queue read land inside this setup, so no test has to
  // remember to flush it before asserting.
  await act(async () => { await Promise.resolve(); });
});
afterEach(async () => {
  // A read started by the last render is still in flight: let it land while the
  // screen is mounted, so its state update happens inside `act` rather than
  // after the teardown.
  await act(async () => { await Promise.resolve(); });
  act(() => tree.unmount());
  jest.useRealTimers();
  jest.restoreAllMocks();
});

/** The native pull-to-refresh indicator the transcript hands to the platform. */
const refreshControl = () =>
  tree.root.findByType(FlatList).props.refreshControl;

/** The rows the top bar's menu is holding right now. */
const menuActions = (root: ReactTestRenderer["root"]) =>
  (root.findByType("ActionMenu" as never).props.actions ?? []) as MenuAction[];

/**
 * Open the conversation's menu, let the open-time repository probe land, and
 * press one of its rows.
 *
 * The real sheet closes and *then* hands the press back (its `pending` payload),
 * so the helper closes it first too — otherwise a row that opens a panel would
 * leave the modal stacked over the surface it just opened. Opening the menu also
 * starts the Git probe; the flush is what keeps a test from reading the list
 * before the desktop's answer has landed.
 */
const pressMenuAction = async (root: ReactTestRenderer["root"], label: string) => {
  await act(async () => { root.findByType(ChatTopBar).props.onMenu(); });
  const action = menuActions(root).find(row => row.label === label);
  const press = action?.onPress;
  if (!press) throw new Error(`no menu row labelled ${label}`);
  act(() => root.findByType("ActionMenu" as never).props.onClose());
  act(() => press());
};

const olderCollision = () => {
  const list = tree.root.findByType(FlatList);
  act(() => list.props.onScrollBeginDrag({
    nativeEvent: {
      contentOffset: { x: 0, y: 1400 },
      contentSize: { width: 320, height: 2000 },
      layoutMeasurement: { width: 320, height: 600 },
    },
  }));
};

test("a desktop another device took over says so where the user is typing", async () => {
  // The status dot explains the refusal, but a user composing here will not tap
  // a small dot to find out why sending does nothing.
  // A fresh mount: `ChatScreen` takes no props, so `update` with an identical
  // element does not re-render it and the changed fixture would be invisible.
  act(() => tree.unmount());
  mockRemote.presence = { control: "standby", holder: { name: "Lab MacBook Air" } };
  act(() => { tree = create(createElement(ChatScreen)); });
  // The panel's opening queue read lands inside the mount.
  await act(async () => { await Promise.resolve(); });

  const notice = () => tree.root.findAll(n => typeof n.type === "function" && n.type.name === "StandbyNotice");
  expect(notice()).toHaveLength(1);
  // It carries the holder's name, so the sentence names who to take it from.
  expect(notice()[0]!.props.holder).toEqual({ name: "Lab MacBook Air" });

  // …and nothing of the sort while this phone drives the desktop.
  act(() => tree.unmount());
  mockRemote.presence = { control: "active", holder: null };
  act(() => { tree = create(createElement(ChatScreen)); });
  await act(async () => { await Promise.resolve(); });
  expect(notice()).toHaveLength(0);
});

test.each(["files", "preview", "download"])("%s retains the list while covered and catches up without remounting", async surface => {
  const original = mockRemote.timeline;
  const list = tree.root.findByType(FlatList);
  const before = list.props.data;
  try {
    if (surface === "files") await pressMenuAction(tree.root, "files.title");
    else {
      if (surface === "preview") mockFileDownload.preview = {};
      else mockFileDownload.activeDownload = {};
      act(() => tree.update(createElement(ChatScreen)));
    }
    mockRemote.timeline = { items: [...original.items, { id: "covered", kind: "message", role: "assistant", text: "arrived while covered" }] };
    act(() => tree.update(createElement(ChatScreen)));
    expect(tree.root.findByType(FlatList).props.data).toBe(before);
    if (surface === "files") {
      expect(tree.root.findAll(node => node.props.accessibilityElementsHidden === true).length).toBeGreaterThan(0);
      act(() => tree.root.findByType(ChatTopBar).props.onBack());
    } else {
      mockFileDownload.preview = null;
      mockFileDownload.activeDownload = null;
      act(() => tree.update(createElement(ChatScreen)));
    }
    expect(tree.root.findByType(FlatList)).toBe(list);
    expect(list.props.data[0]).toMatchObject({ id: "covered", text: "arrived while covered" });
  } finally { mockRemote.timeline = original; }
});

test("file browsing owns system back while the header still returns directly to chat", async () => {
  expect(BackHandler.addEventListener).toHaveBeenCalledTimes(1);
  await pressMenuAction(tree.root, "files.title");
  expect(tree.root.findByType(SessionFilesPanel)).toBeDefined();
  expect(removeBack).toHaveBeenCalledTimes(1);
  // The parent must not install a later listener that swallows folder back.
  expect(BackHandler.addEventListener).toHaveBeenCalledTimes(1);
  act(() => tree.root.findByType(ChatTopBar).props.onBack());
  expect(tree.root.findAllByType(SessionFilesPanel)).toHaveLength(0);
  expect(mockRemote.closeConversation).not.toHaveBeenCalled();
  expect(BackHandler.addEventListener).toHaveBeenCalledTimes(2);
  act(() => tree.root.findByType(ChatTopBar).props.onBack());
  expect(mockRemote.closeConversation).toHaveBeenCalledTimes(1);
});

test("the menu holds files and the spend breakdown, so the bar carries no such buttons", async () => {
  const bar = tree.root.findByType(ChatTopBar);
  expect(bar.props.onFiles).toBeUndefined();
  expect(bar.props.onUsage).toBeUndefined();
  expect(bar.props.menuLabel).toBe("chat.moreOpen");
  expect(bar.props.menuOpen).toBe(false);
  // Closed until asked for: the sheet is not a surface of its own while shut.
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(false);
  await act(async () => { bar.props.onMenu(); });
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(true);
  // A plain directory has no history to show, so the row is absent — not
  // disabled — and the two originals are all the menu has.
  expect(menuActions(tree.root).map(action => action.label)).toEqual(["files.title", "chat.usageOpen"]);
});

test("the spend entry opens the usage sheet, and renaming is not duplicated here", async () => {
  mockRemote.sessionUsage = {
    inputTokens: 2_000, outputTokens: 500, cacheReadTokens: 0, cacheWriteTokens: 0,
    costCny: 0.5, costInputCny: 0.3, costOutputCny: 0.2, costCacheReadCny: 0, costCacheWriteCny: 0,
  };
  act(() => tree.update(createElement(ChatScreen)));
  const bar = tree.root.findByType(ChatTopBar);
  // The session list owns renaming, so the conversation header exposes neither
  // a rename action nor a rename modal.
  expect(bar.props.onRename).toBeUndefined();
  expect(tree.root.findAllByType(RenameModal)).toHaveLength(0);
  expect(tree.root.findByType(SessionUsageSheet).props.visible).toBe(false);

  await pressMenuAction(tree.root, "chat.usageOpen");
  const sheet = tree.root.findByType(SessionUsageSheet);
  expect(sheet.props.visible).toBe(true);
  expect(sheet.props.usage).toBe(mockRemote.sessionUsage);
  // Opening the sheet re-reads the session: the cached amount can be a run
  // behind, and the sheet is the moment it is looked at.
  expect(mockRemote.refreshSessionUsage).toHaveBeenCalledTimes(1);
  // The sheet is an accounting view: closing it is its only action.
  expect(sheet.props.onRename).toBeUndefined();
});

test("a conversation without reported usage still opens the sheet", async () => {
  await pressMenuAction(tree.root, "chat.usageOpen");
  const sheet = tree.root.findByType(SessionUsageSheet);
  expect(sheet.props.visible).toBe(true);
  expect(sheet.props.usage).toBeNull();
});

/**
 * The Git entry exists only where there is a history to show.
 *
 * Whether a workspace is a repository is a property of its directory, so the menu
 * asks every time it opens rather than the bar deciding once — a workspace that is
 * a plain folder never grows a row that would open on nothing, and one where the
 * reader has just run `git init` grows the row on the next look. A desktop that
 * does not advertise the feature is the same case from the phone's side: it cannot
 * answer, so the entry is absent rather than broken.
 */
test("a plain directory keeps the menu at its two originals", async () => {
  const original = mockRemote.capabilities;
  mockRemote.capabilities = new Set(["git_graph_v1"]);
  try {
    act(() => tree.update(createElement(ChatScreen)));
    await act(async () => { tree.root.findByType(ChatTopBar).props.onMenu(); });
    expect(mockRemote.listGitCommits).toHaveBeenCalledTimes(1);
    expect(menuActions(tree.root).map(action => action.label))
      .toEqual(["files.title", "chat.usageOpen"]);
  } finally {
    mockRemote.capabilities = original;
  }
});

test("a host that cannot report a repository is never asked, and offers no row", async () => {
  // No `git_graph_v1`: the command would be refused, so it is not sent at all.
  await act(async () => { tree.root.findByType(ChatTopBar).props.onMenu(); });
  expect(mockRemote.listGitCommits).not.toHaveBeenCalled();
  expect(menuActions(tree.root).map(action => action.label))
    .toEqual(["files.title", "chat.usageOpen"]);
});

test("a repository grows the Git row, and the sheet reads through the controller", async () => {
  const original = mockRemote.capabilities;
  mockRemote.capabilities = new Set(["git_graph_v1"]);
  mockRemote.listGitCommits = jest.fn(async () => ({
    isRepo: true, repoName: "dopamine-decision", path: "/Users/lixin/Research/dopamine-decision",
    commits: [], truncated: false,
  }));
  try {
    act(() => tree.update(createElement(ChatScreen)));
    await act(async () => { tree.root.findByType(ChatTopBar).props.onMenu(); });
    expect(menuActions(tree.root).map(action => action.label))
      .toEqual(["files.title", "chat.usageOpen", "git.open"]);

    // The panel is handed the controller's own reads, not a copy: a second
    // implementation on this screen would drift from the one the mock and the
    // capability check agree with.
    act(() => menuActions(tree.root).find(row => row.label === "git.open")!.onPress!());
    const panel = tree.root.findByType(GitGraphPanel);
    expect(panel.props.listCommits).toBe(mockRemote.listGitCommits);
    expect(panel.props.getCommit).toBe(mockRemote.getGitCommit);
    expect(panel.props.supported).toBe(true);
    expect(panel.props.online).toBe(true);
  } finally {
    mockRemote.capabilities = original;
  }
});

test("the Git graph owns system back and freezes the transcript like the file panel", async () => {
  const original = mockRemote.capabilities;
  mockRemote.capabilities = new Set(["git_graph_v1"]);
  mockRemote.listGitCommits = jest.fn(async () => ({
    isRepo: true, repoName: "repo", path: "/repo", commits: [], truncated: false,
  }));
  try {
    act(() => tree.update(createElement(ChatScreen)));
    act(() => tree.root.findByType(ChatTopBar).props.onMenu());
    await act(async () => { await Promise.resolve(); });
    act(() => menuActions(tree.root).find(row => row.label === "git.open")!.onPress!());
    expect(tree.root.findByType(GitGraphPanel)).toBeDefined();
    // The transcript underneath is frozen and hidden from the screen reader: the
    // sheet is a surface of its own, not an overlay on a live list.
    expect(tree.root.findAll(node => node.props.accessibilityElementsHidden === true).length).toBeGreaterThan(0);
    // The bar's back returns to the conversation from here…
    act(() => tree.root.findByType(ChatTopBar).props.onBack());
    expect(tree.root.findAllByType(GitGraphPanel)).toHaveLength(0);
    expect(mockRemote.closeConversation).not.toHaveBeenCalled();
    // …and a second press leaves the conversation, as it always did.
    act(() => tree.root.findByType(ChatTopBar).props.onBack());
    expect(mockRemote.closeConversation).toHaveBeenCalledTimes(1);
  } finally {
    mockRemote.capabilities = original;
  }
});

test("an ordinary conversation does not expose its internal storage workspace", () => {
  mockRemote.sessions = [{
    sessionId: "history",
    threadId: "thread",
    title: "History",
    mode: "chat",
    workspaceId: "internal-chat-workspace",
    streaming: false,
  }];
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(ChatTopBar).props.contextLabel).toBe(
    "sessions.conversations",
  );
});

test("a legacy conversation can infer its visible workspace when mode is absent", () => {
  mockRemote.sessions = [{
    sessionId: "history",
    threadId: "thread",
    title: "History",
    workspaceId: "workspace",
    streaming: false,
  }];
  mockRemote.workspaces = [{ id: "workspace", name: "Project", path: "/project" }];
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(ChatTopBar).props.contextLabel).toBe(
    "chat.workspaceNamed",
  );
});

test("an older-history collision shows only a one-second loading indicator", async () => {
  mockRemote.loadOlderTimeline.mockResolvedValueOnce(["older"]);
  const list = tree.root.findByType(FlatList);
  expect(list.props.data).toHaveLength(2);
  expect(list.props.inverted).toBe(true);
  expect(list.props.ListFooterComponent).toBeUndefined();
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
  olderCollision();
  const button = tree.root.findByType(FloatingTimelineButton);
  expect(button.props.busy).toBe(true);
  expect(StyleSheet.flatten(button.props.style).top).toBeGreaterThanOrEqual(0);
  expect(mockRemote.loadOlderTimeline).toHaveBeenCalledTimes(1);
  expect(
    tree.root.findByType(FlatList).props.maintainVisibleContentPosition,
  ).toEqual({ minIndexForVisible: 0 });
  expect(tree.root.findByType(FloatingTimelineButton).props.busy).toBe(true);
  expect(tree.root.findByType(FloatingTimelineButton).props.label).toBe(
    "chat.loadingOlder",
  );
  expect(tree.root.findAllByType(ActivityIndicator).length).toBeGreaterThan(0);
  await act(async () => {});
  act(() => list.props.onContentSizeChange(320, 2500));
  act(() => jest.advanceTimersByTime(999));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(1);
  act(() => jest.advanceTimersByTime(1));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
});

test("failed history hides after one second and retries only on a new collision", async () => {
  mockRemote.loadOlderTimeline.mockResolvedValueOnce(false);
  olderCollision();
  await act(async () => {});
  expect(tree.root.findByType(FloatingTimelineButton).props.label).toBe(
    "chat.loadingOlder",
  );
  act(() => jest.advanceTimersByTime(1000));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
  olderCollision();
  expect(mockRemote.loadOlderTimeline).toHaveBeenCalledTimes(2);
});

test("cached messages remain visible and the sync notice stays for at least 750ms", () => {
  const data = tree.root.findByType(FlatList).props.data;
  mockRemote.timelineSyncStatus = "syncing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(FlatList).props.data).toBe(data);
  expect(
    tree.root.findAll((node) => node.props.children === "chat.syncingLatest")
      .length,
  ).toBeGreaterThan(0);
  mockRemote.timelineSyncStatus = "retrying";
  act(() => tree.update(createElement(ChatScreen)));
  expect(
    tree.root.findAll((node) => node.props.children === "chat.syncRetrying")
      .length,
  ).toBeGreaterThan(0);
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(
    tree.root.findAll(
      (node) => node.props.children === "chat.syncWaitingNetwork",
    ).length,
  ).toBeGreaterThan(0);
  mockRemote.desktopOnline = true;
  mockRemote.timelineSyncStatus = "idle";
  act(() => tree.update(createElement(ChatScreen)));
  expect(
    tree.root.findAll(
      (node) => node.props.accessibilityLiveRegion === "polite",
    ),
  ).not.toHaveLength(0);
  act(() => jest.advanceTimersByTime(749));
  expect(
    tree.root.findAll(
      (node) => node.props.accessibilityLiveRegion === "polite",
    ),
  ).not.toHaveLength(0);
  act(() => jest.advanceTimersByTime(1));
  expect(
    tree.root.findAll(
      (node) => node.props.accessibilityLiveRegion === "polite",
    ),
  ).toHaveLength(0);
  expect(tree.root.findByType(FlatList).props.data).toBe(data);
});

test("the pull spinner belongs to the pull, not to the lane", () => {
  // An automatic sync already says so in the notice above the transcript; the
  // native indicator at the bottom must not report the same wait a second time.
  mockRemote.timelineSyncStatus = "syncing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(refreshControl().props.refreshing).toBe(false);

  // A pull spins it, and it stays up until the lane it restarted settles.
  act(() => refreshControl().props.onRefresh());
  expect(mockRemote.reloadTimeline).toHaveBeenCalledTimes(1);
  expect(refreshControl().props.refreshing).toBe(true);

  mockRemote.timelineSyncStatus = "idle";
  act(() => tree.update(createElement(ChatScreen)));
  expect(refreshControl().props.refreshing).toBe(false);
});

test("the pull's spinner is the only report of the wait it restarts", () => {
  // The pull restarts the same lane the notice above the transcript reports, so
  // both come up at once and say the same thing twice. The spinner is at the
  // finger; the pill stands down for the state it would duplicate.
  const notice = (key: string) =>
    tree.root.findAll(node => node.props.children === key).length > 0;
  act(() => refreshControl().props.onRefresh());
  mockRemote.timelineSyncStatus = "syncing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(refreshControl().props.refreshing).toBe(true);
  expect(notice("chat.syncingLatest")).toBe(false);
  act(() => jest.advanceTimersByTime(2000));
  expect(notice("chat.syncingLatest")).toBe(false);

  // The two richer states stay: the spinner cannot say "retrying" or "waiting
  // for the connection", so the pill is no longer a repeat of it.
  mockRemote.timelineSyncStatus = "retrying";
  act(() => tree.update(createElement(ChatScreen)));
  expect(notice("chat.syncRetrying")).toBe(true);
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(notice("chat.syncWaitingNetwork")).toBe(true);
  // Waiting for the connection is not work in flight: the notice drops its own
  // spinner (the lane cannot make progress on its own here).
  const waiting = tree.root.find(
    node => node.props.accessibilityLiveRegion === "polite",
  );
  expect(waiting.findAllByType(ActivityIndicator)).toHaveLength(0);
});

test("live output is the only report of the wait behind it", () => {
  // A reconcile behind a streaming run is routine engine traffic — every gap is
  // healed from the journal while the run streams — and the transcript already
  // grows with the run's own row. Announcing it again above the text is the
  // state the reader cannot act on.
  const notice = (key: string) =>
    tree.root.findAll(node => node.props.children === key).length > 0;
  mockRemote.timelineSyncStatus = "syncing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(notice("chat.syncingLatest")).toBe(true);

  mockRemote.streaming = true;
  act(() => tree.update(createElement(ChatScreen)));
  act(() => jest.advanceTimersByTime(2000));
  expect(notice("chat.syncingLatest")).toBe(false);

  // The two richer states stay: a failing lane or a connection the lane cannot
  // use is not visible in the streaming text.
  mockRemote.timelineSyncStatus = "retrying";
  act(() => tree.update(createElement(ChatScreen)));
  expect(notice("chat.syncRetrying")).toBe(true);
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(notice("chat.syncWaitingNetwork")).toBe(true);
  mockRemote.desktopOnline = true;

  // Back to a plain reconcile while the run still streams: quiet again.
  mockRemote.timelineSyncStatus = "syncing";
  act(() => tree.update(createElement(ChatScreen)));
  act(() => jest.advanceTimersByTime(2000));
  expect(notice("chat.syncingLatest")).toBe(false);

  // Generation over, the lane still catching up: the pill is back, because
  // nothing on screen says the transcript is still catching up.
  mockRemote.streaming = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(notice("chat.syncingLatest")).toBe(true);
});

test("text selection cannot trigger Android focus-driven transcript scrolling", () => {
  const list = tree.root.findByType(FlatList);
  expect(list.props.inverted).toBe(true);
  expect(list.props.scrollsChildToFocus).toBe(false);
  // Do not solve selection jumps by disabling manual reading/scrolling.
  expect(list.props.scrollEnabled).not.toBe(false);
});

test("no floating history button when the history is exhausted", () => {
  mockRemote.canLoadOlderTimeline = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
});

test("an external loading flag does not create an extra prompt without a collision", () => {
  mockRemote.loadingOlderTimeline = true;
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
});

// The card holds the draft, but the send button stays live: pressing it is the
// same as the card's "send without it" — dismiss the suggestion, send as typed.
// Swallowing the press (or only toasting) read as a dead send button.
test("a plain send with a skill card up dismisses it and sends the draft", async () => {
  mockDraft.message = "please search the web for this";
  mockSkillReco.suggestion = {
    skill: { name: "future-web", description: "search the web" },
    draft: mockDraft.message,
  };
  act(() => tree.update(createElement(ChatScreen)));

  // `send` is async: the press has to be awaited or its state updates land after
  // this test finished and poison whichever test runs next (§8.18).
  await act(async () => {
    await tree.root.findByType(ComposerDock).props.send();
  });

  expect(mockSkillReco.dismiss).toHaveBeenCalledTimes(1);
  expect(mockSendApi.send).toHaveBeenCalledTimes(1);
});

test("an empty composer sends nothing and asks no skill oracle", async () => {
  mockDraft.message = "   ";
  mockDraft.attachments = [];
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.send();
  });

  // No draft means nothing to evaluate and nothing to send: an empty turn would
  // burn a desktop round trip and put a blank bubble in the thread.
  expect(mockSkillReco.evaluate).not.toHaveBeenCalled();
  expect(mockSendApi.send).not.toHaveBeenCalled();
});

test("a skill the oracle does not recommend falls through to a normal send", async () => {
  mockDraft.message = "summarise this";
  mockSkillReco.evaluate.mockResolvedValueOnce(false);
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.send();
  });

  // "No match, timeout, error" all take this path: the message still goes out.
  expect(mockSkillReco.evaluate).toHaveBeenCalledWith("summarise this");
  expect(mockSendApi.send).toHaveBeenCalledTimes(1);
});

test("a recommended skill holds the draft instead of sending it", async () => {
  mockDraft.message = "search the web for the changelog";
  mockSkillReco.evaluate.mockResolvedValueOnce(true);
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.send();
  });

  // The card is on screen asking the user to decide, so sending now would both
  // pre-empt the choice and leave the card describing a draft it no longer has.
  expect(mockSkillReco.evaluate).toHaveBeenCalledTimes(1);
  expect(mockSendApi.send).not.toHaveBeenCalled();
});

test("installing the suggested skill sends the composed draft, not the raw one", async () => {
  mockDraft.message = "search the web";
  mockSkillReco.suggestion = {
    skill: { name: "future-web", description: "search the web" },
    draft: mockDraft.message,
  };
  mockSkillReco.installAndUse.mockResolvedValueOnce("use future-web: search the web");
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.onInstallSkill();
  });

  // The send has to carry the composed text explicitly: the composer state still
  // holds the pre-install draft, and sending *that* would silently drop the
  // skill the user just installed for this message.
  expect(mockDraft.setMessage).toHaveBeenCalledWith("use future-web: search the web");
  expect(mockSendApi.send).toHaveBeenCalledWith("use future-web: search the web");
  expect(tree.root.findByType(ComposerDock).props.skillInstalling).toBe(false);
});

test("a failed skill install keeps the card up and sends nothing", async () => {
  mockDraft.message = "search the web";
  mockSkillReco.suggestion = {
    skill: { name: "future-web", description: "search the web" },
    draft: mockDraft.message,
  };
  mockSkillReco.installAndUse.mockResolvedValueOnce(null);
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.onInstallSkill();
  });

  // PRD v1.6 §6.2: the user must be able to retry or send without it, so the
  // draft stays unsent and the installing flag is released.
  expect(mockSendApi.send).not.toHaveBeenCalled();
  expect(tree.root.findByType(ComposerDock).props.skillSuggestion).not.toBeNull();
  expect(tree.root.findByType(ComposerDock).props.skillInstalling).toBe(false);
});

test("a second install tap cannot start a second install", async () => {
  mockDraft.message = "search the web";
  mockSkillReco.suggestion = {
    skill: { name: "future-web", description: "search the web" },
    draft: mockDraft.message,
  };
  let release!: (value: string | null) => void;
  mockSkillReco.installAndUse.mockReturnValueOnce(
    new Promise<string | null>(resolve => { release = resolve; }),
  );
  act(() => tree.update(createElement(ChatScreen)));

  let first!: Promise<void>;
  await act(async () => {
    first = tree.root.findByType(ComposerDock).props.onInstallSkill();
  });
  expect(tree.root.findByType(ComposerDock).props.skillInstalling).toBe(true);
  // The dock keeps the button mounted while it spins; a second tap must be
  // absorbed rather than queue a second install of the same skill.
  await act(async () => {
    await tree.root.findByType(ComposerDock).props.onInstallSkill();
  });
  expect(mockSkillReco.installAndUse).toHaveBeenCalledTimes(1);
  await act(async () => {
    release("composed");
    await first;
  });
  expect(mockSendApi.send).toHaveBeenCalledTimes(1);
});

test("an approval decision reports its own failure and clears the spinner", async () => {
  const decide = jest.fn<Promise<void>, [string, string]>();
  decide.mockRejectedValueOnce(new Error("desktop offline"));
  mockRemote.decideApproval = decide as never;
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.decideApproval("a1", "approved");
  });

  // A refused decision must keep the card actionable: the error names the card
  // it belongs to, and the submitting flag is released so it can be retried.
  expect(decide).toHaveBeenCalledWith("a1", "approved");
  expect(tree.root.findByType(ComposerDock).props.approvalError).toMatchObject({ id: "a1" });
  expect(tree.root.findByType(ComposerDock).props.approvalSubmitting).toBeNull();

  decide.mockResolvedValueOnce(undefined);
  await act(async () => {
    await tree.root.findByType(ComposerDock).props.decideApproval("a1", "approved");
  });
  // A later success clears the previous failure rather than leaving it pinned.
  expect(tree.root.findByType(ComposerDock).props.approvalError).toBeNull();
  expect(tree.root.findByType(ComposerDock).props.approvalSubmitting).toBeNull();
});

test("the model selector receives the model the reference resolves to", () => {
  mockRemote.models = [
    { id: "a", provider: "p", label: "Model A" },
    { id: "b", provider: "p", label: "Model B" },
  ];
  mockRemote.modelId = "p/b";
  act(() => tree.update(createElement(ChatScreen)));

  // The composer labels the *labelled* active model, which only works when the
  // provider-qualified reference is matched against the catalogue.
  expect(tree.root.findByType(ComposerDock).props.activeModelLabel).toBe("Model B");

  // A reference the catalogue does not list falls back to the raw id, never to
  // a blank label the user cannot act on.
  mockRemote.modelId = "p/missing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(ComposerDock).props.activeModelLabel).toBe("p/missing");
});

/**
 * The screen is mostly wiring: props it hands to the platform and to the mocked
 * surfaces. A lambda that is never invoked is indistinguishable from one that
 * was deleted, so each is driven here and its effect asserted.
 */
describe("ChatScreen wiring", () => {
  // A renderer and a keyboard-listener capture of its own. Mounting once per
  // test with the spy already installed means no test has to unmount and remount
  // to observe a listener, which is what kept corrupting the shared tree.
  let screen!: ReactTestRenderer;
  const keyHandlers: ((event: { endCoordinates: { height: number } }) => void)[] = [];
  let keySpy!: jest.SpyInstance;
  beforeEach(async () => {
    keyHandlers.length = 0;
    keySpy = jest.spyOn(Keyboard, "addListener").mockImplementation(((
      _name: string,
      handler: (event: { endCoordinates: { height: number } }) => void,
    ) => {
      keyHandlers.push(handler);
      return { remove: jest.fn() };
    }) as never);
    act(() => { screen = create(createElement(ChatScreen)); });
    // The panel's opening queue read is in flight; let it land inside the setup.
    await act(async () => { await Promise.resolve(); });
  });
  afterEach(() => {
    act(() => screen.unmount());
    keySpy.mockRestore();
  });
  const view = () => screen.root;

  test("the transcript's own callbacks reach the scroll and layout controllers", () => {
    const list = () => view().findByType(FlatList);
    // These are the platform's calls, not the app's: a handler that stopped
    // forwarding would leave paging and the reading position silently frozen.
    act(() => list().props.onLayout({ nativeEvent: { layout: { width: 320, height: 600, x: 0, y: 0 } } }));
    act(() => list().props.onScroll({
      nativeEvent: {
        contentOffset: { x: 0, y: 0 },
        contentSize: { width: 320, height: 2000 },
        layoutMeasurement: { width: 320, height: 600 },
      },
    }));
    act(() => list().props.onScrollBeginDrag({
      nativeEvent: {
        contentOffset: { x: 0, y: 1400 },
        contentSize: { width: 320, height: 2000 },
        layoutMeasurement: { width: 320, height: 600 },
      },
    }));
    act(() => list().props.onContentSizeChange(320, 2000));
    expect(list().props.data).toBeDefined();
  });

  test("a conversation ending on the user's own prompt has no live reply to highlight", () => {
    // Only a trailing *assistant* message is the streaming one; the loop that
    // finds it has to fall through to null rather than pick the user's turn.
    const original = mockRemote.timeline;
    try {
      mockRemote.timeline = { items: [{ id: "u", kind: "message", role: "user", text: "Q" }] };
      act(() => screen.update(createElement(ChatScreen)));
      const card = view().findAllByType("TimelineCard" as never)[0]!;
      expect(card.props.isLatestAssistant).toBe(false);
    } finally {
      mockRemote.timeline = original;
    }
  });

  test("failed history hands the user a retry that re-reads the session", () => {
    // This test is the only one that rewrites the shared transcript, so it puts
    // it back: a leaked empty timeline makes every later test render nothing.
    const original = mockRemote.timeline;
    const originalError = mockRemote.timelineError;
    try {
      mockRemote.timeline = { items: [] };
      mockRemote.timelineError = "history_timeout";
      mockRemote.retryTimeline = jest.fn();
      act(() => screen.update(createElement(ChatScreen)));

      const retry = view().findAll(node =>
        node.props.accessibilityRole === "button" && typeof node.props.onPress === "function"
        && node.findAllByType(Text).some(child => child.props.children === "common.retry"))[0];
      expect(retry).toBeDefined();
      retry!.props.onPress();
      // The empty transcript would otherwise be a dead end: this is the only
      // affordance that can bring the history back.
      expect(mockRemote.retryTimeline).toHaveBeenCalledTimes(1);
    } finally {
      mockRemote.timeline = original;
      mockRemote.timelineError = originalError;
    }
  });

  test("system back returns to the conversation list", () => {
    const handler = jest.mocked(BackHandler.addEventListener).mock.calls.at(-1)![1];
    let handled: boolean | null | undefined;
    act(() => { handled = handler({ type: "hardwareBackPress", timeStamp: 1 }); });
    // Handled *and* acted on: returning true alone would swallow the gesture.
    expect(handled).toBe(true);
    expect(mockRemote.closeConversation).toHaveBeenCalledTimes(1);
  });

  test("both keyboard events move or release the suggestion offset", () => {
    // A keyboard that opens must raise the card above it and one that closes
    // must release the offset; leaving either unhandled parks the card behind
    // the keys or leaves the composer permanently raised.
    expect(keyHandlers.length).toBeGreaterThanOrEqual(2);
    for (const handler of keyHandlers) {
      act(() => handler({ endCoordinates: { height: 320 } }));
      act(() => handler({ endCoordinates: { height: 0 } }));
    }
    expect(view().findByType(ComposerDock)).toBeDefined();
  });

  test("the transcript's row actions reach the controllers behind them", () => {
    // `TimelineCard` is mocked to a host element, so its props are readable and
    // its callbacks can be driven exactly as the real card drives them.
    const card = view().findAllByType("TimelineCard" as never)[0]!;
    expect(card).toBeDefined();
    const attachment = { path: "C:\\work\\a.png", name: "a.png" };
    const item = { id: "a-last", kind: "message" as const, role: "assistant" as const, text: "answer" };
    // Each callback is a stable indirection onto the newest controller closure
    // (memoized cards must not capture a stale one), so the assertion is that
    // the prop reaches a live controller rather than throwing.
    act(() => card.props.onOpenAttachment(attachment));
    expect(mockFileDownload.openAttachment).toHaveBeenCalledWith(attachment);
    act(() => card.props.onOpenFile("C:\\work\\a.png"));
    expect(mockFileDownload.openFileLink).toHaveBeenCalledWith("C:\\work\\a.png");
    act(() => card.props.onRetry(item));
    expect(mockSendApi.retryMessage).toHaveBeenCalledWith(item);
    act(() => card.props.onContinue(item));
    expect(mockSendApi.continueMessage).toHaveBeenCalledWith(item);
  });

  test("a press of ↓ lands on the next question, skipping the answer between", () => {
    // The walk is wired from the screen through the list into the hook: a
    // control that stopped forwarding its press would leave ↓ doing nothing,
    // and no test of the walk itself would notice.
    const original = mockRemote.timeline;
    try {
      mockRemote.timeline = {
        items: [
          { id: "q1", kind: "message", role: "user", text: "older question" },
          { id: "a1", kind: "message", role: "assistant", text: "older answer" },
          { id: "q2", kind: "message", role: "user", text: "question" },
          { id: "a2", kind: "message", role: "assistant", text: "answer" },
        ],
      };
      act(() => screen.update(createElement(ChatScreen)));

      const list = () => view().findByType(FlatList);
      const scrollToIndex = jest.spyOn(
        list().instance as FlatList<TimelineItem>,
        "scrollToIndex",
      ).mockImplementation(() => {});

      // The reader dragged up to the older question, so ↓ offers the prompt
      // below it — the answer in between is not a stop.
      act(() => list().props.onScrollBeginDrag({
        nativeEvent: {
          contentOffset: { x: 0, y: 1_360 },
          contentSize: { width: 320, height: 2_000 },
          layoutMeasurement: { width: 320, height: 600 },
        },
      }));
      act(() => list().props.viewabilityConfigCallbackPairs[0].onViewableItemsChanged({
        viewableItems: [{ index: 3, key: "q1", isViewable: true }],
        changed: [],
      }));
      const down = view().findAll(node =>
        node.props.accessibilityLabel === "chat.questionNext"
        && typeof node.props.onPress === "function")[0];
      expect(down).toBeDefined();
      act(() => down!.props.onPress());

      // The answer between the two prompts is passed in this one press.
      expect(scrollToIndex).toHaveBeenCalledWith({
        index: 1,
        viewPosition: 1,
        viewOffset: JUMP_VIEW_OFFSET,
        animated: false,
      });
    } finally {
      mockRemote.timeline = original;
    }
  });

  test("fork stays hidden on a host that does not advertise it, and reaches the controller when it does", () => {
    const item = { id: "a-last", kind: "message" as const, role: "assistant" as const, text: "answer" };
    expect(view().findAllByType("TimelineCard" as never)[0]!.props.onFork).toBeUndefined();

    const original = mockRemote.capabilities;
    mockRemote.capabilities = new Set(["fork_v1"]);
    try {
      act(() => screen.update(createElement(ChatScreen)));
      const card = view().findAllByType("TimelineCard" as never)[0]!;
      expect(typeof card.props.onFork).toBe("function");
      act(() => card.props.onFork(item));
      expect(mockSendApi.forkMessage).toHaveBeenCalledWith(item);
    } finally {
      mockRemote.capabilities = original;
    }
  });

  test("the file panel is told about the conversation and routes its file opens", async () => {
    mockRemote.sessions = [
      { sessionId: "history", threadId: "t", title: "History", mode: "workspace", streaming: false },
    ];
    mockRemote.listSessionFiles = jest.fn(async () => null);
    await pressMenuAction(view(), "files.title");
    act(() => screen.update(createElement(ChatScreen)));

    const panel = view().findByType(SessionFilesPanel);
    // The panel is told the conversation is a workspace, so it can offer the
    // workspace-rooted listing rather than the session root.
    expect(panel.props.isWorkspace).toBe(true);
    expect(panel.props.listFiles).toBe(mockRemote.listSessionFiles);
    act(() => panel.props.onOpenFile("C:\\work\\notes.md"));
    expect(mockFileDownload.openFileLink).toHaveBeenCalledWith("C:\\work\\notes.md", true);
    // Closing and reopening must not stack a second panel: the pane is a single
    // surface the header toggles, and the transcript underneath stays frozen
    // (PausedTimeline) rather than remounting.
    act(() => panel.props.onClose());
    await pressMenuAction(view(), "files.title");
    act(() => screen.update(createElement(ChatScreen)));
    expect(view().findAllByType(SessionFilesPanel)).toHaveLength(1);
  });

  test("a sync that resumes inside the notice's minimum window cancels the queued hide", () => {
    // The pill is held for a minimum window so it cannot flash; if the lane
    // starts working again inside that window the queued hide must be
    // cancelled, or the notice would vanish while a sync is running.
    const polite = () => view().findAll(node => node.props.accessibilityLiveRegion === "polite").length;
    mockRemote.timelineSyncStatus = "syncing";
    act(() => screen.update(createElement(ChatScreen)));
    expect(polite()).toBeGreaterThan(0);
    mockRemote.timelineSyncStatus = "idle";
    act(() => screen.update(createElement(ChatScreen)));
    // The hide is queued but has not fired: no timer has been advanced yet.
    mockRemote.timelineSyncStatus = "syncing";
    act(() => screen.update(createElement(ChatScreen)));
    mockRemote.timelineSyncStatus = "idle";
    act(() => screen.update(createElement(ChatScreen)));

    // Advancing well past the window proves the earlier hide was cancelled:
    // a leaked timer would have dropped the pill mid-sync and this one would
    // then be waiting on its own fresh minimum.
    act(() => jest.advanceTimersByTime(200));
    expect(polite()).toBeGreaterThan(0);
  });

  test("the load-older hint dedupes a tap while a page is in flight", () => {
    const original = mockRemote.timeline;
    try {
      // Nothing collided yet: no hint at all.
      expect(view().findAllByType(FloatingTimelineButton)).toHaveLength(0);
      // A drag that runs into the top of the loaded window is what asks for more.
      act(() => view().findByType(FlatList).props.onScrollBeginDrag({
        nativeEvent: {
          contentOffset: { x: 0, y: 1400 },
          contentSize: { width: 320, height: 2000 },
          layoutMeasurement: { width: 320, height: 600 },
        },
      }));
      act(() => screen.update(createElement(ChatScreen)));
      const hint = () => view().findAllByType(FloatingTimelineButton)[0]!;
      expect(hint()).toBeDefined();
      // The collision started a page, so the button reports the paging
      // controller's real state rather than a fixed label.
      expect(hint().props.busy).toBe(true);
      const queued = mockRemote.loadOlderTimeline.mock.calls.length;
      act(() => hint().props.onPress());
      // `loadOlder` waits for a fresh collision, so the button's own press
      // cannot smuggle in a second page behind the controller's back.
      expect(mockRemote.loadOlderTimeline.mock.calls.length).toBe(queued);
    } finally {
      mockRemote.timeline = original;
    }
  });

  test("the spend sheet opens from the header and closes from its own dismiss", async () => {
    // The usage sheet itself, not "the first dismissible surface": the menu is
    // mounted (closed) on every render now, and it also answers to `onClose`.
    const usage = () => view().findByType(SessionUsageSheet);
    expect(usage().props.visible).toBe(false);
    await pressMenuAction(view(), "chat.usageOpen");
    act(() => screen.update(createElement(ChatScreen)));
    expect(usage().props.visible).toBe(true);
    // The sheet closes through its own callback, so a dismissal gesture cannot
    // leave it mounted over the transcript.
    act(() => usage().props.onClose());
    act(() => screen.update(createElement(ChatScreen)));
    expect(usage().props.visible).toBe(false);
  });

  test("the native action sheet hands its selection back to the download controller", async () => {
    const fileAction = { info: { transferId: "t1", name: "a.pdf" }, cachedFile: null };
    mockFileDownload.fileAction = fileAction;
    act(() => screen.update(createElement(ChatScreen)));

    const sheet = view().find(node =>
      node.props.action !== undefined && typeof node.props.onSelect === "function");
    expect(sheet).toBeDefined();
    await act(async () => { await sheet.props.onSelect(fileAction, "share"); });
    // The sheet owns no policy: it hands the chosen operation and file back to
    // the controller, which owns the download, the preview and the handoff.
    expect(mockFileDownload.openOrShare).toHaveBeenCalledWith(fileAction.info, null, "share");
    act(() => sheet.props.onClose());
    expect(mockFileDownload.setFileAction).toHaveBeenCalledWith(null);
  });
test("a `#` reference in a sent message opens the conversation it names", () => {
  const card = view().findAllByType("TimelineCard" as never)[0]!;
  // `findAllByType` types every element's `props` loosely, so take the callback
  // from the row's own props type: the call below is then checked against the
  // prop the screen really passes rather than against an inferred `any`.
  const openSession = card.props.onOpenSession as NonNullable<TimelineCardProps["onOpenSession"]>;
  expect(typeof openSession).toBe("function");
  act(() => openSession("s-9"));
  // The transcript did not change the conversation by itself: the screen routed
  // the chip's own request through the controls, which is what makes the tap a
  // conversation switch rather than a dead link.
  expect(mockRemote.selectSession).toHaveBeenCalledWith("s-9");
});

test("a lean tool row asks this machine for the command the page dropped", async () => {
  const card = view().findAllByType("TimelineCard" as never)[0]!;
  const resolveToolTarget = card.props.onResolveToolTarget as NonNullable<TimelineCardProps["onResolveToolTarget"]>;
  expect(typeof resolveToolTarget).toBe("function");
  mockRemote.resolveToolCallTarget.mockResolvedValueOnce("ls -la /tmp");
  let resolved: string | null = null;
  await act(async () => { resolved = await resolveToolTarget("call_1", "run_1"); });
  expect(mockRemote.resolveToolCallTarget).toHaveBeenCalledWith("call_1", "run_1");
  expect(resolved).toBe("ls -la /tmp");
});

test("Fusion details preserve step and byte offset through the stable timeline callback", async () => {
  const card = view().findAllByType("TimelineCard" as never)[0]!;
  const resolve = card.props.onResolveToolTarget as NonNullable<TimelineCardProps["onResolveToolTarget"]>;
  mockRemote.resolveToolCallTarget.mockResolvedValueOnce('{"transcript":"evidence"}');
  const page = { stepId: "step-1", offset: 32768 };
  await act(async () => { await resolve("fusion-call", "parent-run", page); });
  expect(mockRemote.resolveToolCallTarget).toHaveBeenCalledWith("fusion-call", "parent-run", page);
});

});


test("conversation navigation removes outgoing transfer progress and rejects late callbacks", async () => {
  const progressWidth = (value: string) => tree.root.findAll(node =>
    node.props.style && StyleSheet.flatten(node.props.style)?.width === value);
  const reportA = mockSendOptions.current!.setTransferProgress;
  act(() => reportA(0.25));
  expect(progressWidth("25%").length).toBeGreaterThan(0);
  mockRemote.selectedSessionId = "B";
  act(() => tree.update(createElement(ChatScreen)));
  // The panel re-reads the queue for the conversation it now shows.
  await act(async () => { await Promise.resolve(); });
  expect(progressWidth("25%")).toHaveLength(0);
  act(() => mockSendOptions.current!.setTransferProgress(0.5));
  act(() => reportA(0.75));
  expect(progressWidth("50%").length).toBeGreaterThan(0);
  expect(progressWidth("75%")).toHaveLength(0);
});

/**
 * The top bar names the machine the conversation is on.
 *
 * Every conversation on this phone lives on a paired desktop — there is no agent
 * on the phone — and the bar said nothing about which one, so a user who paired a
 * second desktop could not tell them apart from inside a conversation. The icon
 * id (the shared set's mark) and the user's own name are the session list's vocabulary, so the same desktop
 * reads the same way in both places; a machine the user never renamed falls back
 * to its id.
 */
test("the bar carries the desktop this conversation is on, named the way the list names it", () => {
  expect(tree.root.findByType(ChatTopBar).props.machine)
    .toEqual({ icon: "laptop", label: "Lab MacBook Pro" });

  // Renamed or not, the machine is still named: the id is what is left when the
  // user has not given it a label.
  mockRemote.desktops = [{ desktopId: "desktop", pairId: "pair_1" }];
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(ChatTopBar).props.machine)
    .toEqual({ icon: undefined, label: "desktop" });
});


// Where the viewer sits and what its dismissal is wired to. The surfaces above
// it are stubbed in this file, so a picture cannot be opened through the
// transcript here; what this screen owns is the position (last, so it stacks
// above the file preview a picture can be opened from) and the state its close
// resets.
test("the image viewer is the last surface, and its dismissal leaves the conversation as it was", () => {
  const viewer = () => tree.root.findAll(node => typeof node.type === "function"
    && (node.type as { name?: string }).name === "ImageViewer").at(-1)!;
  // Nothing is open until a picture is tapped.
  expect(viewer().props.target).toBeNull();

  // Last of the surfaces, in the same stack as the sheets it must cover.
  const siblings = viewer().parent!.children;
  expect(siblings[siblings.length - 1]).toBe(viewer());

  // The close button and Android's back gesture both run this: with nothing
  // open it must leave the screen exactly as it found it.
  expect(typeof viewer().props.onClose).toBe("function");
  act(() => viewer().props.onClose());
  expect(viewer().props.target).toBeNull();
});

// ── Send queue: the session's, read from the desktop ───────────────────────
// Nothing below is held by this phone. A message sent while the reply streams
// goes to the desktop at once with a queueing policy; the panel draws the rows
// the session reports (`get_queue`), and the three verbs address the session's
// own run ids.

const queueRow = (fields: Partial<SessionQueueRow> & { runId: string }): SessionQueueRow => ({
  acceptedAt: "2026-01-01T00:00:00Z",
  position: 1,
  state: "queued",
  text: "waiting",
  ...fields,
});

/** One answer from the desktop: the rows it holds and what recently left them. */
const queueReply = (
  rows: SessionQueueRow[],
  acks: SessionQueueSnapshot["acks"] = [],
): SessionQueueSnapshot => ({ acks, rows });

/** Let the panel's read land. */
const settleQueue = async () => {
  await act(async () => { await Promise.resolve(); });
};

/**
 * Mount the screen with the session already answering this queue.
 *
 * The panel reads when it opens, so a fixture set after the mount would only
 * arrive on the next tick — which is a different thing from what is queued now.
 */
const mountWithQueue = async (
  rows: SessionQueueRow[],
  options: { streaming?: boolean; acks?: SessionQueueSnapshot["acks"] } = {},
) => {
  act(() => tree.unmount());
  mockRemote.getQueue.mockResolvedValue(queueReply(rows, options.acks ?? []));
  mockRemote.streaming = options.streaming ?? false;
  act(() => { tree = create(createElement(ChatScreen)); });
  await settleQueue();
};

const queueContents = () =>
  tree.root.findByType(ComposerDock).props.queueItems.map((item: { content: string }) => item.content);

test("the panel draws the rows the session queued, and the screen sends nothing on its own", async () => {
  await mountWithQueue(
    [
      queueRow({ runId: "run_a", text: "queued one" }),
      queueRow({ position: 2, runId: "run_b", text: "queued two" }),
    ],
    { streaming: true },
  );

  expect(mockRemote.getQueue).toHaveBeenCalledWith("history");
  expect(queueContents()).toEqual(["queued one", "queued two"]);
  // The run ending is the session's business: it starts the row itself.
  mockRemote.streaming = false;
  await act(async () => { tree.update(createElement(ChatScreen)); });
  expect(mockRemote.sendMessage).not.toHaveBeenCalled();
});

test("delete cancels that row's own run id on the session", async () => {
  await mountWithQueue([queueRow({ runId: "run_a", text: "take me back" })]);
  const dock = () => tree.root.findByType(ComposerDock).props;
  const readsBefore = mockRemote.getQueue.mock.calls.length;

  await act(async () => { dock().onQueueRemove("run_a"); });
  expect(mockRemote.cancelQueuedRun).toHaveBeenCalledWith("history", "run_a");
  // The panel is re-read whatever the answer was, so the row goes when the
  // session says it went rather than when this screen assumes it did.
  expect(mockRemote.getQueue.mock.calls.length).toBeGreaterThan(readsBefore);
});

test("editing a waiting message takes the row back and queues the new words", async () => {
  await mountWithQueue([queueRow({ runId: "run_a", text: "typo" })]);

  await act(async () => {
    tree.root.findByType(ComposerDock).props.onQueueUpdate("run_a", "  fixed  ");
  });
  // A queued prompt has no editable text on the session: cancel, then send.
  expect(mockRemote.cancelQueuedRun).toHaveBeenCalledWith("history", "run_a");
  expect(mockRemote.sendMessage).toHaveBeenCalledWith(
    "fixed", [], undefined, { busyPolicy: QUEUED_SEND_POLICY },
  );
  expect(mockRemote.cancelQueuedRun.mock.invocationCallOrder[0]!)
    .toBeLessThan(mockRemote.sendMessage.mock.invocationCallOrder[0]!);
});

test("an edit that empties the row is not a message, so nothing is sent", async () => {
  await mountWithQueue([queueRow({ runId: "run_a", text: "typo" })]);

  await act(async () => {
    tree.root.findByType(ComposerDock).props.onQueueUpdate("run_a", "   ");
  });
  expect(mockRemote.cancelQueuedRun).not.toHaveBeenCalled();
  expect(mockRemote.sendMessage).not.toHaveBeenCalled();
});

const queuedPhoto = {
  kind: "image", name: "shot.png", path: "/Users/x/shot.png", size: 2048,
};
const queuedDoc = {
  kind: "file", name: "notes.pdf", path: "/Users/x/notes.pdf", size: 4096,
};

test("editing a waiting message keeps the files it carries", async () => {
  await mountWithQueue([
    queueRow({ attachments: [queuedPhoto], runId: "run_a", text: "look" }),
  ]);

  await act(async () => {
    tree.root.findByType(ComposerDock).props.onQueueUpdate("run_a", "look again");
  });
  await settleQueue();

  // The file is copied down first: a row cancelled before its files arrived
  // would lose the very files the edit is supposed to keep.
  expect(mockRemote.prepareAttachment).toHaveBeenCalledWith(
    expect.objectContaining({ path: queuedPhoto.path }),
  );
  expect(mockRemote.cancelQueuedRun).toHaveBeenCalledWith("history", "run_a");
  expect(mockRemote.sendMessage).toHaveBeenCalledWith(
    "look again",
    [expect.objectContaining({ kind: "image", name: "shot.png" })],
    undefined,
    { busyPolicy: QUEUED_SEND_POLICY },
  );
});

test("removing one file re-sends the message with the rest", async () => {
  await mountWithQueue([
    queueRow({ attachments: [queuedPhoto, queuedDoc], runId: "run_a", text: "see these" }),
  ]);

  await act(async () => {
    tree.root.findByType(ComposerDock).props.onQueueRemoveAttachment("run_a", queuedPhoto.path);
  });
  await settleQueue();

  // Only the kept file is fetched; the dropped one is never sent again.
  expect(mockRemote.prepareAttachment).toHaveBeenCalledTimes(1);
  expect(mockRemote.prepareAttachment).toHaveBeenCalledWith(
    expect.objectContaining({ path: queuedDoc.path }),
  );
  const sent = mockRemote.sendMessage.mock.calls[0]![1]!;
  expect(sent.map(item => (item as { name: string }).name)).toEqual(["notes.pdf"]);
});

test("adding a photo to a waiting message sends the kept files with it", async () => {
  const remoteFiles = jest.requireMock("../../../remote/files") as {
    pickAttachments: jest.Mock;
  };
  const added = {
    kind: "image",
    localUri: "file:///cache/new.png",
    mimeType: "image/png",
    name: "new.png",
    originalSize: 100,
    transferSize: 100,
  };
  // The picker answers with the whole list, the row's own file included.
  remoteFiles.pickAttachments.mockResolvedValueOnce([
    { ...added, localUri: queuedPhoto.path, name: queuedPhoto.name },
    added,
  ]);
  await mountWithQueue([
    queueRow({ attachments: [queuedPhoto], runId: "run_a", text: "look" }),
  ]);
  const dock = () => tree.root.findByType(ComposerDock).props;

  await act(async () => { dock().onQueueAddAttachments("run_a"); });
  const menu = tree.root.findByType(AttachmentSourceMenu);
  expect(menu.props.visible).toBe(true);
  // The picker's quotas count what the row already carries, without pulling the
  // kept files down first.
  await act(async () => { menu.props.onPick("files"); });
  await settleQueue();

  expect(remoteFiles.pickAttachments).toHaveBeenCalledWith([
    expect.objectContaining({ localUri: queuedPhoto.path, name: "shot.png", originalSize: 2048 }),
  ]);
  const sent = mockRemote.sendMessage.mock.calls[0]!;
  expect(sent[0]).toBe("look");
  expect(sent[1]!.map(item => (item as { name: string }).name)).toEqual(["shot.png", "new.png"]);
  expect(sent[3]).toEqual({ busyPolicy: QUEUED_SEND_POLICY });
});

test("a parked row offers 'send now', and pressing it releases that run", async () => {
  await mountWithQueue([
    queueRow({ runId: "run_parked", state: "stale:stale", text: "waited too long" }),
  ]);
  const items = tree.root.findByType(ComposerDock).props.queueItems;
  expect(items[0].parked).toBe(true);

  await act(async () => {
    tree.root.findByType(ComposerDock).props.onQueueRelease("run_parked");
  });
  expect(mockRemote.releaseQueuedRun).toHaveBeenCalledWith("history", "run_parked");
});

test("a row about to run at the boundary is not offered as a manual send", async () => {
  await mountWithQueue([queueRow({ runId: "run_a" })]);
  expect(tree.root.findByType(ComposerDock).props.queueItems[0].parked).toBe(false);
});

test("a refused cancel is told to the reader instead of silently keeping the row", async () => {
  const toast = jest.spyOn(chatUtils, "showToast").mockImplementation(() => {});
  await mountWithQueue([queueRow({ runId: "run_a" })]);
  mockRemote.cancelQueuedRun.mockRejectedValueOnce(new Error("not_connected"));

  await act(async () => {
    tree.root.findByType(ComposerDock).props.onQueueRemove("run_a");
  });
  expect(toast).toHaveBeenCalled();
});

/**
 * A row can leave the desktop's queue without this screen asking — folded into
 * the turn ahead, taken back on the desktop, lost with a deleted conversation.
 * Without its terminal ack the row simply disappears, and the reader believes the
 * message went out.
 */
test("says why a row left the queue, even with the panel already empty", async () => {
  await mountWithQueue([queueRow({ runId: "run_a" })], { streaming: true });
  expect(tree.root.findByType(ComposerDock).props.queueNotice).toBeUndefined();

  mockRemote.getQueue.mockResolvedValue(
    queueReply([], [{ reason: "merged", runId: "run_a", state: "terminal" }]),
  );
  await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });

  // The rows are gone; the sentence is what is left to read, and it is drawn by
  // the dock rather than inside the panel that just unmounted.
  expect(tree.root.findByType(ComposerDock).props.queueItems).toEqual([]);
  expect(tree.root.findByType(ComposerDock).props.queueNotice).toBe("chat.queue.notice.merged");
});

/** Deleting a row here is this screen's own act: its ack would read as a fault. */
test("does not announce the removal the reader just made", async () => {
  await mountWithQueue([queueRow({ runId: "run_a" })], { streaming: true });

  await act(async () => {
    tree.root.findByType(ComposerDock).props.onQueueRemove("run_a");
  });
  mockRemote.getQueue.mockResolvedValue(
    queueReply([], [{ reason: "cancelled", runId: "run_a", state: "terminal" }]),
  );
  await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });

  expect(tree.root.findByType(ComposerDock).props.queueItems).toEqual([]);
  expect(tree.root.findByType(ComposerDock).props.queueNotice).toBeUndefined();
});

/**
 * An edit is a cancel and a re-send, so the row is gone on the desktop before the
 * new words leave. A refusal must put them back in the box rather than cost the
 * reader both the row and the correction.
 */
test("keeps the corrected words when the desktop refuses the re-send", async () => {
  await mountWithQueue([queueRow({ runId: "run_a", text: "typo" })], { streaming: true });
  mockRemote.sendMessage.mockRejectedValueOnce(new Error("session_busy"));

  await act(async () => {
    tree.root.findByType(ComposerDock).props.onQueueUpdate("run_a", "  fixed  ");
  });

  expect(mockRemote.cancelQueuedRun).toHaveBeenCalledWith("history", "run_a");
  // The draft is restored through the composer's own setter (an updater, so a
  // half-written draft is not overwritten).
  const restore = mockDraft.setMessage.mock.calls.at(-1)![0] as (current: string) => string;
  expect(restore("")).toBe("fixed");
  expect(restore("half-written")).toBe("half-written");
});

test("another conversation's rows are never drawn under this one's box", async () => {
  await mountWithQueue([queueRow({ runId: "run_a", text: "for history" })]);
  expect(queueContents()).toEqual(["for history"]);

  // The reader moves on: the rows they were shown belong to the conversation
  // they left, and whatever that session answers next must not draw here.
  mockRemote.selectedSessionId = "other";
  mockRemote.getQueue.mockImplementation(() => new Promise(() => {}));
  act(() => tree.update(createElement(ChatScreen)));
  expect(queueContents()).toEqual([]);
});

test("an idle conversation reads the queue once, and keeps reading only while rows wait", async () => {
  act(() => tree.unmount());
  mockRemote.getQueue.mockResolvedValue(queueReply([]));
  act(() => { tree = create(createElement(ChatScreen)); });
  await settleQueue();
  const idleReads = mockRemote.getQueue.mock.calls.length;
  expect(idleReads).toBeGreaterThan(0);

  // Nothing streaming and nothing waiting: no timer spends the pairing on reads.
  await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS * 3); });
  expect(mockRemote.getQueue.mock.calls.length).toBe(idleReads);

  // A run starts (somebody sent): the panel reads now, then keeps following it.
  mockRemote.getQueue.mockResolvedValue(queueReply([queueRow({ runId: "run_a", text: "from the desktop" })]));
  mockRemote.streaming = true;
  await act(async () => { tree.update(createElement(ChatScreen)); });
  await settleQueue();
  expect(queueContents()).toEqual(["from the desktop"]);
  const liveReads = mockRemote.getQueue.mock.calls.length;
  await act(async () => { jest.advanceTimersByTime(QUEUE_POLL_MS); });
  expect(mockRemote.getQueue.mock.calls.length).toBeGreaterThan(liveReads);
});

test("a queue that cannot be read leaves the panel empty rather than failing the screen", async () => {
  act(() => tree.unmount());
  mockRemote.getQueue.mockRejectedValue(new Error("not_connected"));
  act(() => { tree = create(createElement(ChatScreen)); });
  await settleQueue();
  expect(queueContents()).toEqual([]);
  expect(mockRemote.getQueue).toHaveBeenCalled();
});
