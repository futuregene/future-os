import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import App from "../../../App";
import { SessionsScreen } from "../../screens/SessionsScreen";
import { ChatScreen } from "../../features/chat/ChatScreen";

const mockRemote = { phase: "ready", credentials: { pairId: "pair" }, desktops: [], selectedSessionId: "", draft: false };
jest.mock("../../remote/RemoteContext", () => ({ RemoteProvider: ({ children }: { children: unknown }) => children, useRemoteControls: () => mockRemote }));
jest.mock("expo-status-bar", () => ({ StatusBar: () => null }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaProvider: ({ children }: { children: unknown }) => children }));
jest.mock("../../screens/SessionsScreen", () => ({ SessionsScreen: () => null }));
jest.mock("../../features/chat/ChatScreen", () => ({ ChatScreen: () => null }));
jest.mock("../../screens/DesktopsScreen", () => ({ DesktopsScreen: () => null }));
jest.mock("../../screens/PairingScreen", () => ({ PairingScreen: () => null }));
jest.mock("../../share/ShareIntakeMenu", () => ({ ShareIntakeMenu: () => null }));
jest.mock("../../share/shareInbox", () => ({ subscribeShareLanded: () => () => {}, shareLandedRevision: () => 0 }));
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
