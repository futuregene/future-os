import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import App from "../../../App";
import { SessionsScreen } from "../../screens/SessionsScreen";
import { ChatScreen } from "../../features/chat/ChatScreen";
import { DesktopsScreen } from "../../screens/DesktopsScreen";
import { PairingScreen } from "../../screens/PairingScreen";
import { markShareLanded } from "../../share/shareInbox";

const mockRemote: {
  phase: string;
  credentials: { pairId: string } | null;
  desktops: { desktopId: string; pairId: string }[];
  connectionPresentation: { level: string };
  selectedSessionId: string;
  draft: boolean;
} = {
  phase: "ready",
  credentials: { pairId: "pair" },
  desktops: [{ desktopId: "desktop", pairId: "pair" }],
  connectionPresentation: { level: "connected" },
  selectedSessionId: "",
  draft: false,
};
jest.mock("../../remote/RemoteContext", () => ({ RemoteProvider: ({ children }: { children: unknown }) => children, useRemoteControls: () => mockRemote }));
jest.mock("expo-status-bar", () => ({ StatusBar: () => null }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaProvider: ({ children }: { children: unknown }) => children }));
jest.mock("../../screens/SessionsScreen", () => ({ SessionsScreen: () => null }));
jest.mock("../../features/chat/ChatScreen", () => ({ ChatScreen: () => null }));
jest.mock("../../screens/DesktopsScreen", () => ({ DesktopsScreen: () => null }));
jest.mock("../../screens/PairingScreen", () => ({ PairingScreen: () => null }));
jest.mock("../../share/ShareIntakeMenu", () => ({ ShareIntakeMenu: () => null }));
jest.mock("../../update/useUpdateReminder", () => ({ useUpdateReminder: () => {} }));
let mockLanguageReady = true;
jest.mock("../../i18n/useSystemLanguage", () => ({ useSystemLanguage: () => mockLanguageReady }));

test("waits for the saved language before mounting app screens", () => {
  mockLanguageReady = false;
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(App)); });
  expect(tree.root.findAllByType(SessionsScreen)).toHaveLength(0);
  mockLanguageReady = true;
  act(() => tree.update(createElement(App)));
  expect(tree.root.findAllByType(SessionsScreen)).toHaveLength(1);
  act(() => tree.unmount());
});

test("opens the desktop picker when pairings exist without an active credential", () => {
  mockRemote.credentials = null;
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(App)); });
  expect(tree.root.findAllByType(DesktopsScreen)).toHaveLength(1);
  expect(tree.root.findAllByType(SessionsScreen)).toHaveLength(0);
  expect(tree.root.findAllByType(PairingScreen)).toHaveLength(0);
  expect(tree.root.findByType(DesktopsScreen).props.onBack).toBeUndefined();
  act(() => tree.unmount());
  mockRemote.credentials = { pairId: "pair" };
});

test("opens pairing only when there are no saved desktops", () => {
  const desktops = mockRemote.desktops;
  mockRemote.credentials = null;
  mockRemote.desktops = [];
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(App)); });
  expect(tree.root.findAllByType(PairingScreen)).toHaveLength(1);
  expect(tree.root.findAllByType(DesktopsScreen)).toHaveLength(0);
  act(() => tree.unmount());
  mockRemote.credentials = { pairId: "pair" };
  mockRemote.desktops = desktops;
});

test.each(["", "existing-session"])("a share remounts the composer even when destination %s is already open", sessionId => {
  mockRemote.selectedSessionId = sessionId;
  mockRemote.draft = !sessionId;
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(App)); });
  try {
    const composer = tree.root.findByType(ChatScreen);
    act(() => markShareLanded());
    expect(tree.root.findByType(ChatScreen)).not.toBe(composer);
    expect(mockRemote.selectedSessionId).toBe(sessionId);
  } finally {
    act(() => tree.unmount());
    mockRemote.selectedSessionId = "";
    mockRemote.draft = false;
  }
});

test("opening and closing a chat retains the list instance and disables its background handlers", () => {
  jest.useFakeTimers();
  let tree: ReactTestRenderer;
  act(() => { tree = create(createElement(App)); });
  try {
    const list = tree!.root.findByType(SessionsScreen);
    for (let round = 0; round < 5; round++) {
      mockRemote.selectedSessionId = "session";
      act(() => tree!.update(createElement(App)));
      expect(tree!.root.findByType(SessionsScreen)).toBe(list);
      expect(list.props.active).toBe(false);
      expect(tree!.root.findAllByType(ChatScreen)).toHaveLength(1);
      mockRemote.selectedSessionId = "";
      act(() => tree!.update(createElement(App)));
      expect(tree!.root.findByType(SessionsScreen)).toBe(list);
      expect(list.props.active).toBe(true);
      expect(tree!.root.findAllByType(ChatScreen)).toHaveLength(0);
    }
  } finally {
    act(() => tree!.unmount());
    jest.useRealTimers();
    mockRemote.selectedSessionId = "";
  }
});
