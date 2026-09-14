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
import { ChatScreen } from "../ChatScreen";
import { FloatingTimelineButton } from "../components/FloatingTimelineButton";
import type { TimelineSyncStatus } from "../../../remote/syncEngine";
import type { RemoteSession, RemoteWorkspace } from "../../../remote/types";

const mockRemote = {
  credentials: { expectedDesktopId: "desktop" },
  selectedSessionId: "history",
  selectedTitle: "History",
  closeConversation: jest.fn(),
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
jest.mock("../useFileDownload", () => ({ useFileDownload: () => ({}) }));
jest.mock("../useSendMessage", () => ({ useSendMessage: () => ({}) }));
jest.mock("../useRename", () => ({ useRename: () => ({}) }));
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
  mockRemote.canLoadOlderTimeline = true;
  mockRemote.desktopOnline = true;
  mockRemote.timelineSyncStatus = "idle";
  mockRemote.loadingOlderTimeline = false;
  mockRemote.sessions = [];
  mockRemote.workspaces = [];
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
