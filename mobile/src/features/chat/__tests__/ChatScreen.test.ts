import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { BackHandler, FlatList, Pressable } from "react-native";
import { ChatTopBar } from "../components/ChatTopBar";
import { SessionFilesPanel } from "../components/SessionFilesPanel";
import { ChatScreen } from "../ChatScreen";
import type { TimelineSyncStatus } from "../../../remote/syncEngine";

const mockRemote = {
  credentials: { expectedDesktopId: "desktop" },
  selectedSessionId: "history",
  selectedTitle: "History",
  closeConversation: jest.fn(),
  sessions: [], workspaces: [], models: [], capabilities: new Set(),
  timeline: { items: [
    { id: "u-last", kind: "message", role: "user", text: "Question" },
    { id: "a-last", kind: "message", role: "assistant", text: "Short answer" },
  ] },
  canLoadOlderTimeline: true,
  loadingOlderTimeline: false,
  loadOlderTimeline: jest.fn<Promise<string[]>, []>(),
  desktopOnline: true,
  timelineSyncStatus: "idle" as TimelineSyncStatus,
  connectionPresentation: { level: "connected" },
};
jest.mock("../../../remote/RemoteContext", () => ({
  useRemote: () => mockRemote,
  useRemoteControls: () => mockRemote,
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView", useSafeAreaInsets: () => ({ bottom: 0 }) }));
jest.mock("lucide-react-native", () => ({ History: "History" }));
jest.mock("../../../components/TimelineCard", () => ({ TimelineCard: "TimelineCard" }));
jest.mock("../../../components/ErrorBanner", () => ({ ErrorBanner: "ErrorBanner" }));
jest.mock("../useComposerDraft", () => ({ useComposerDraft: () => ({ message: "", attachments: [] }) }));
jest.mock("../useAttachmentPicker", () => ({ useAttachmentPicker: () => ({}) }));
jest.mock("../useFileDownload", () => ({ useFileDownload: () => ({}) }));
jest.mock("../useSendMessage", () => ({ useSendMessage: () => ({}) }));
jest.mock("../useRename", () => ({ useRename: () => ({}) }));
jest.mock("../components/ChatTopBar", () => ({ ChatTopBar: "ChatTopBar" }));
jest.mock("../components/SessionFilesPanel", () => ({ SessionFilesPanel: "SessionFilesPanel" }));
jest.mock("../components/ComposerDock", () => ({ ComposerDock: "ComposerDock" }));
jest.mock("../components/ModelSelectorSheet", () => ({ ModelSelectorSheet: "ModelSelectorSheet" }));
jest.mock("../components/DownloadProgressModal", () => ({ DownloadProgressModal: "DownloadProgressModal" }));
jest.mock("../components/PreviewModal", () => ({ PreviewModal: "PreviewModal" }));
jest.mock("../components/RenameModal", () => ({ RenameModal: "RenameModal" }));
jest.mock("../components/NativeFileActionSheet", () => ({ NativeFileActionSheet: "NativeFileActionSheet" }));

let tree: ReactTestRenderer;
const removeBack = jest.fn();
beforeEach(() => {
  jest.clearAllMocks();
  jest.spyOn(BackHandler, "addEventListener").mockReturnValue({ remove: removeBack });
  jest.useFakeTimers();
  mockRemote.canLoadOlderTimeline = true;
  mockRemote.desktopOnline = true;
  mockRemote.timelineSyncStatus = "idle";
  mockRemote.loadingOlderTimeline = false;
  mockRemote.loadOlderTimeline.mockReset().mockResolvedValue([]);
  act(() => { tree = create(createElement(ChatScreen)); });
});
afterEach(() => {
  act(() => tree.unmount());
  jest.useRealTimers();
  jest.restoreAllMocks();
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

test("a single short exchange exposes a clickable older-history footer before any scrolling", async () => {
  const list = tree.root.findByType(FlatList);
  expect(list.props.data).toHaveLength(2);
  expect(list.props.inverted).toBe(true);
  const footer = list.props.ListFooterComponent;
  expect(footer.type).toBe(Pressable);
  expect(footer.props.disabled).toBe(false);
  await act(async () => { footer.props.onPress(); });
  expect(mockRemote.loadOlderTimeline).toHaveBeenCalledTimes(1);
  expect(tree.root.findByType(FlatList).props.ListFooterComponent.props.disabled).toBe(true);
  act(() => jest.advanceTimersByTime(100));
  expect(tree.root.findByType(FlatList).props.ListFooterComponent.props.disabled).toBe(false);
});

test("cached messages remain visible with a sync notice until replay is complete", () => {
  const data = tree.root.findByType(FlatList).props.data;
  mockRemote.timelineSyncStatus = "syncing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(FlatList).props.data).toBe(data);
  expect(tree.root.findAll(node => node.props.children === "chat.syncingLatest").length).toBeGreaterThan(0);
  mockRemote.timelineSyncStatus = "retrying";
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findAll(node => node.props.children === "chat.syncRetrying").length).toBeGreaterThan(0);
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findAll(node => node.props.children === "chat.syncWaitingNetwork").length).toBeGreaterThan(0);
  mockRemote.desktopOnline = true;
  mockRemote.timelineSyncStatus = "idle";
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findAll(node => node.props.accessibilityLiveRegion === "polite")).toHaveLength(0);
  expect(tree.root.findByType(FlatList).props.data).toBe(data);
});

test("no older-history footer when the history is exhausted", () => {
  mockRemote.canLoadOlderTimeline = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(FlatList).props.ListFooterComponent).toBeNull();
});

test("an external history load disables the footer", () => {
  mockRemote.loadingOlderTimeline = true;
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(FlatList).props.ListFooterComponent.props.disabled).toBe(true);
});
