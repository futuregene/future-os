import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import {
  ActivityIndicator,
  BackHandler,
  FlatList,
  StyleSheet,
} from "react-native";
import { ChatTopBar } from "../components/ChatTopBar";
import { SessionFilesPanel } from "../components/SessionFilesPanel";
import { SessionUsageSheet } from "../components/SessionUsageSheet";
import { RenameModal } from "../components/RenameModal";
import { ChatScreen } from "../ChatScreen";
import { FloatingTimelineButton } from "../components/FloatingTimelineButton";
import type { TimelineSyncStatus } from "../../../remote/syncEngine";
import type { RemoteSession, RemoteSessionUsage, RemoteWorkspace } from "../../../remote/types";

const mockRemote = {
  credentials: { expectedDesktopId: "desktop" },
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
  models: [],
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
  desktopOnline: true,
  timelineSyncStatus: "idle" as TimelineSyncStatus,
  connectionPresentation: { level: "connected" },
};
jest.mock("../../../remote/RemoteContext", () => ({
  useRemote: () => mockRemote,
  useRemoteControls: () => mockRemote,
}));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ bottom: 0 }),
}));
jest.mock("lucide-react-native", () => ({ History: "History" }));
jest.mock("../../../components/TimelineCard", () => ({
  TimelineCard: "TimelineCard",
}));
jest.mock("../../../components/ErrorBanner", () => ({
  ErrorBanner: "ErrorBanner",
}));
jest.mock("../useComposerDraft", () => ({
  useComposerDraft: () => ({ message: "", attachments: [] }),
}));
jest.mock("../useAttachmentPicker", () => ({
  useAttachmentPicker: () => ({}),
}));
const mockFileDownload: { preview: unknown; activeDownload: unknown } = { preview: null, activeDownload: null };
jest.mock("../useFileDownload", () => ({ useFileDownload: () => mockFileDownload }));
jest.mock("../useSendMessage", () => ({ useSendMessage: () => ({}) }));
jest.mock("../components/SessionUsageSheet", () => ({
  SessionUsageSheet: "SessionUsageSheet",
}));
jest.mock("../components/ChatTopBar", () => ({ ChatTopBar: "ChatTopBar" }));
jest.mock("../components/SessionFilesPanel", () => ({
  SessionFilesPanel: "SessionFilesPanel",
}));
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
beforeEach(() => {
  jest.clearAllMocks();
  jest
    .spyOn(BackHandler, "addEventListener")
    .mockReturnValue({ remove: removeBack });
  jest.useFakeTimers();
  mockFileDownload.preview = null;
  mockFileDownload.activeDownload = null;
  mockRemote.canLoadOlderTimeline = true;
  mockRemote.desktopOnline = true;
  mockRemote.timelineSyncStatus = "idle";
  mockRemote.loadingOlderTimeline = false;
  mockRemote.sessions = [];
  mockRemote.workspaces = [];
  mockRemote.sessionUsage = null;
  mockRemote.loadOlderTimeline.mockReset().mockResolvedValue([]);
  act(() => {
    tree = create(createElement(ChatScreen));
  });
});
afterEach(() => {
  act(() => tree.unmount());
  jest.useRealTimers();
  jest.restoreAllMocks();
});

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

test.each(["files", "preview", "download"])("%s retains the list while covered and catches up without remounting", surface => {
  const original = mockRemote.timeline;
  const list = tree.root.findByType(FlatList);
  const before = list.props.data;
  try {
    if (surface === "files") act(() => tree.root.findByType(ChatTopBar).props.onFiles());
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

test("file browsing owns system back while the header still returns directly to chat", () => {
  expect(BackHandler.addEventListener).toHaveBeenCalledTimes(1);
  act(() => tree.root.findByType(ChatTopBar).props.onFiles());
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

test("the spend icon opens the usage sheet, and renaming is not duplicated here", () => {
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

  act(() => bar.props.onUsage());
  const sheet = tree.root.findByType(SessionUsageSheet);
  expect(sheet.props.visible).toBe(true);
  expect(sheet.props.usage).toBe(mockRemote.sessionUsage);
  // Opening the sheet re-reads the session: the cached amount can be a run
  // behind, and the sheet is the moment it is looked at.
  expect(mockRemote.refreshSessionUsage).toHaveBeenCalledTimes(1);
  // The sheet is an accounting view: closing it is its only action.
  expect(sheet.props.onRename).toBeUndefined();
});

test("a conversation without reported usage still opens the sheet", () => {
  act(() => tree.root.findByType(ChatTopBar).props.onUsage());
  const sheet = tree.root.findByType(SessionUsageSheet);
  expect(sheet.props.visible).toBe(true);
  expect(sheet.props.usage).toBeNull();
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
