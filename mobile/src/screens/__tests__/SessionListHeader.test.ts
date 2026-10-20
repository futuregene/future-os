import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Text, TextInput } from "react-native";
import { SessionsScreen } from "../SessionsScreen";
import { SessionList } from "../SessionList";
import { ActionMenu } from "../../components/ActionMenu";
import type { MachineSummary } from "../../remote/desktopStatus";

/**
 * The header's two page tools, wired to the list they act on.
 *
 * `SessionsScreen.test.ts` draws the screen with the list stubbed out, which is
 * how its own chrome is tested — but the tools in that header are only half a
 * feature without the list: search and batch selection open *there*. So this
 * file keeps the real list and asks the one question the other file cannot:
 * does the header's menu item actually reach it?
 */

const mockRemote = {
  desktops: [{ desktopId: "desktop_1", pairId: "p1", name: "Studio" }],
  credentials: { pairId: "p1", expectedDesktopId: "desktop_1" },
  workspaces: [] as { id: string; name: string; path: string }[],
  sessions: [
    { sessionId: "s1", threadId: "t1", title: "First", streaming: false },
    { sessionId: "s2", threadId: "t2", title: "Second", streaming: false },
  ],
  unreadSessions: new Set<string>(),
  connectionPresentation: {
    level: "connected",
    titleKey: "connection.connected",
    hintKey: "connection.offlineHint",
  },
  desktopOnline: true,
  capabilities: new Set<string>(),
  desktopSettingsRevision: 0,
  skillsRevision: 0,
  phase: "connected",
  hasConnectedContent: true,
  error: null,
  presence: null,
  catalogs: {
    catalogs: [],
    machines: [] as MachineSummary[],
    refresh: async () => {},
    reconnect: () => {},
    commandsFor: () => null,
    takeControlFor: async () => undefined,
    readWorkspaces: async () => [],
  },
  newConversation: jest.fn(),
  newConversationOn: jest.fn(async () => {}),
  reconnect: jest.fn(),
  takeControl: jest.fn(async () => {}),
  clearError: jest.fn(),
  selectSession: jest.fn(),
  switchDesktop: jest.fn(async () => {}),
  selectSessionOnDesktop: jest.fn(async () => {}),
  openSessionOnDesktop: jest.fn(async () => {}),
  deleteSession: jest.fn(async () => {}),
  rename: jest.fn(async () => {}),
  generateTitle: jest.fn(async () => "Generated"),
  setSessionPinned: jest.fn(async () => {}),
  unpair: jest.fn(async () => {}),
  renameWorkspace: jest.fn(async () => {}),
  setWorkspacePinned: jest.fn(async () => {}),
  deleteWorkspace: jest.fn(async () => {}),
  createWorkspace: jest.fn(async () => ({ id: "w9", name: "New", path: "/host/new" })),
  refreshWorkspaces: jest.fn(async () => {}),
  refreshModels: jest.fn(async () => {}),
  getDesktopSettings: jest.fn(async () => ({})),
  listSettingsModels: jest.fn(async () => []),
  listInstalledSkills: jest.fn(async () => []),
  listAvailableSkills: jest.fn(async () => []),
};
jest.mock("../../remote/RemoteContext", () => ({
  useRemoteControls: () => mockRemote,
  useRemote: () => mockRemote,
}));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { name?: string; title?: string; count?: number }) =>
      options?.title
        ? `${key}:${options.title}`
        : options?.count !== undefined
          ? `${key}:${options.count}`
          : options?.name
            ? `${key}:${options.name}`
            : key,
    i18n: { language: "en" },
  }),
}));
jest.mock("lucide-react-native", () => Object.fromEntries(
  [
    "ArrowLeft", "Check", "CheckCheck", "ChevronDown", "ChevronLeft", "ChevronRight",
    "CircleAlert", "Folder", "Layers", "ListChecks", "LogOut", "MessageCircle", "Minus",
    "Monitor", "MonitorOff", "MoreHorizontal", "Pencil", "Pin", "Plus", "Search",
    "Settings", "Trash2", "Unplug", "X",
  ].map(name => [name, name]),
));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("../../i18n/LanguageSettings", () => ({ LanguageSettings: () => null }));
jest.mock("../../update/prompt", () => ({ promptUpgrade: jest.fn() }));
jest.mock("../../update/update", () => ({ checkForUpdate: jest.fn() }));
jest.mock("../../remote/peerIcons", () => ({ iconGlyph: () => "" }));
const mockStorage = new Map<string, string>();
jest.mock("@react-native-async-storage/async-storage", () => ({
  __esModule: true,
  default: {
    getItem: jest.fn(async (key: string) => mockStorage.get(key) ?? null),
    setItem: jest.fn(async (key: string, value: string) => {
      mockStorage.set(key, value);
    }),
    removeItem: jest.fn(async (key: string) => {
      mockStorage.delete(key);
    }),
  },
}));

let tree: ReactTestRenderer;
const onManageDesktops = jest.fn();
const button = (label: string) => tree.root.findAll(node =>
  node.props.accessibilityLabel === label && typeof node.props.onPress === "function",
)[0]!;
/** The header's own sheet, which is the one this file is about. */
const headerMenu = () => tree.root
  .findAllByType(ActionMenu)
  .find(node => node.props.title === "sessions.listActions")!;

beforeEach(async () => {
  jest.clearAllMocks();
  act(() => {
    tree = create(createElement(SessionsScreen, { onManageDesktops }));
  });
  // The folds read the store on mount; let it land inside act().
  await act(async () => { await Promise.resolve(); });
});
afterEach(() => act(() => tree.unmount()));

const openHeaderMenu = () => act(() => button("sessions.listActions").props.onPress());
const item = (label: string) => headerMenu().findAll(node =>
  node.props.accessibilityLabel === label && typeof node.props.onPress === "function",
)[0]!;
/**
 * Take one action from the header's sheet.
 *
 * A sheet's item only *queues* its action — the sheet has to come down first —
 * so the dismissal is the other half of the tap, exactly as on a device.
 */
const pressItem = (label: string) => {
  const action = item(label);
  act(() => action.props.onPress());
  act(() => { headerMenu().findByType(Modal).props.onDismiss(); });
};

test("the header's search item opens the list's search field", () => {
  expect(tree.root.findAllByType(TextInput)).toHaveLength(0);
  openHeaderMenu();
  pressItem("sessions.search");
  // The field replaces the page bar, and the header — machine, status dots,
  // settings — stays where it was, which is why the field lives in the list.
  const input = tree.root.findByType(TextInput);
  expect(input.props.autoFocus).toBe(true);
  expect(tree.root.findAll(node => node.props.accessibilityRole === "tab")).toHaveLength(0);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === "sessions.settings").length)
    .toBeGreaterThan(0);
});

test("the header's selection item enters a batch on the catalogued rows", () => {
  // The real list, not a stub: this is the wiring the stubbed screen cannot see.
  expect(tree.root.findAllByType(SessionList)).toHaveLength(1);
  openHeaderMenu();
  pressItem("sessions.select");
  // The batch bar is up and holds nothing until rows are ticked; the page bar is
  // gone with it, so no swipe can change page unseen.
  expect(tree.root.findAll(node => node.props.accessibilityLabel === "sessions.selectVisible").length)
    .toBeGreaterThan(0);
  expect(tree.root.findAll(node => node.props.accessibilityRole === "tab")).toHaveLength(0);
  // And it is the real list's own bar: its rows are tickable.
  const row = tree.root.findAll(node =>
    node.props.accessibilityRole === "button" && typeof node.props.onLongPress === "function",
  ).find(node => node.findAllByType(Text).some(text => text.props.children === "First"))!;
  act(() => row.props.onPress());
  expect(tree.root.findAll(node => node.props.children === "sessions.selectedCount:1").length)
    .toBeGreaterThan(0);
});

test("the selection item is unavailable when nothing may join a batch", () => {
  // Pinned conversations are shortcuts no batch may hold, so a catalogue of
  // nothing else has nothing to select — read from the catalogue, not from the
  // rows on screen.
  mockRemote.sessions = mockRemote.sessions.map(session => ({ ...session, pinned: true }));
  act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
  openHeaderMenu();
  expect(item("sessions.select").props.disabled).toBe(true);
  mockRemote.sessions = [
    { sessionId: "s1", threadId: "t1", title: "First", streaming: false },
    { sessionId: "s2", threadId: "t2", title: "Second", streaming: false },
  ];
});

test("a machine that cannot answer offers no batch to hold", () => {
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
  openHeaderMenu();
  expect(item("sessions.select").props.disabled).toBe(true);
  mockRemote.desktopOnline = true;
});
