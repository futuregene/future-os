import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { FlatList, Pressable } from "react-native";
import { ChatScreen } from "../ChatScreen";

const mockRemote = {
  credentials: { expectedDesktopId: "desktop" },
  selectedSessionId: "history",
  selectedTitle: "History",
  sessions: [], workspaces: [], models: [], capabilities: new Set(),
  timeline: { items: [
    { id: "u-last", kind: "message", role: "user", text: "Question" },
    { id: "a-last", kind: "message", role: "assistant", text: "Short answer" },
  ] },
  canLoadOlderTimeline: true,
  loadingOlderTimeline: false,
  loadOlderTimeline: jest.fn<Promise<string[]>, []>(),
  desktopOnline: true,
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
beforeEach(() => {
  jest.useFakeTimers();
  mockRemote.canLoadOlderTimeline = true;
  mockRemote.loadingOlderTimeline = false;
  mockRemote.loadOlderTimeline.mockReset().mockResolvedValue([]);
  act(() => { tree = create(createElement(ChatScreen)); });
});
afterEach(() => {
  act(() => tree.unmount());
  jest.useRealTimers();
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
