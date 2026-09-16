import { createElement, type Dispatch, type SetStateAction } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { RemoteProvider, useRemote, useRemoteControls } from "../RemoteContext";
import type { TimelineState } from "../timeline";
import type { RemoteSessionState, StreamEvent } from "../types";

const mockCatalog = { sessions: [], models: [], workspaces: [], unreadSessions: new Set(), observeRunEvent: jest.fn() };
const mockConnection = { phase: "ready", desktopOnline: true, presence: { agentAvailable: true } };
const mockConversation = { modelId: "model", thinkingLevel: "off", openingSession: false,
  applySessionSettings: jest.fn(), handleSessionSettingsEvent: jest.fn(),
};
const mockOutbox = { sending: false };
let mockSetTimeline: Dispatch<SetStateAction<TimelineState>>;
let mockHandleEvent: (event: StreamEvent, sessionId: string) => void;
let mockOnSessionState: (sessionId: string, state: RemoteSessionState) => void;
jest.mock("../useSessionCatalog", () => ({ useSessionCatalog: () => mockCatalog }));
jest.mock("../useRemoteConnection", () => ({ useRemoteConnection: (options: { handleEvent: typeof mockHandleEvent }) => {
  mockHandleEvent = options.handleEvent;
  return mockConnection;
} }));
jest.mock("../useConversationController", () => ({ useConversationController: () => mockConversation }));
jest.mock("../usePromptOutbox", () => ({ usePromptOutbox: () => mockOutbox }));
jest.mock("../useTimelineController", () => ({
  useTimelineController: (options: { onSessionState: typeof mockOnSessionState }) => {
    mockOnSessionState = options.onSessionState;
    const React = jest.requireActual<typeof import("react")>("react");
    const { emptyTimeline } = jest.requireActual<typeof import("../timeline")>("../timeline");
    const [timeline, setTimeline] = React.useState(emptyTimeline);
    mockSetTimeline = setTimeline;
    return { timeline, timelinePending: false, timelineError: null, canLoadOlderTimeline: false, loadingOlderTimeline: false, handleEvent: jest.fn() };
  },
}));

test("provider wires live settings and complete recovery snapshots to the conversation controls", () => {
  let renderer!: ReactTestRenderer;
  act(() => { renderer = create(createElement(RemoteProvider)); });
  try {
    const event = { type: "model_changed", data: '{"model":"p/new"}' };
    act(() => mockHandleEvent(event, "s1"));
    expect(mockConversation.handleSessionSettingsEvent).toHaveBeenLastCalledWith(event, "s1");
    act(() => mockOnSessionState("s1", { model: "p/new", thinkingLevel: "high" }));
    expect(mockConversation.applySessionSettings).toHaveBeenLastCalledWith("s1", { model: "p/new", thinkingLevel: "high" });
    // A deleted/empty session must not inherit the last conversation's model.
    act(() => mockOnSessionState("empty", {}));
    expect(mockConversation.applySessionSettings).toHaveBeenLastCalledWith("empty", { model: "", thinkingLevel: "off" });
  } finally { act(() => renderer.unmount()); }
});

test("100 text commits do not re-render controls, but streaming state changes do", () => {
  let controlRenders = 0;
  let transcriptRenders = 0;
  let streaming = false;
  function Controls() {
    streaming = useRemoteControls().streaming;
    controlRenders++;
    return null;
  }
  function Transcript() {
    useRemote();
    transcriptRenders++;
    return null;
  }
  let renderer!: ReactTestRenderer;
  act(() => {
    renderer = create(createElement(RemoteProvider, null, createElement(Controls), createElement(Transcript)));
  });
  try {
    const initialControls = controlRenders;
    const initialTranscript = transcriptRenders;
    for (let i = 0; i < 100; i++) {
      act(() => mockSetTimeline(previous => ({
        ...previous, items: [{ kind: "message", id: "assistant", role: "assistant", text: `frame ${i}` }],
      })));
    }
    expect(controlRenders).toBe(initialControls);
    expect(transcriptRenders).toBe(initialTranscript + 100);
    act(() => mockSetTimeline(previous => ({ ...previous, streaming: true })));
    expect(streaming).toBe(true);
    expect(controlRenders).toBe(initialControls + 1);
    act(() => mockSetTimeline(previous => ({ ...previous, streaming: false })));
    expect(streaming).toBe(false);
    expect(controlRenders).toBe(initialControls + 2);
  } finally { act(() => renderer.unmount()); }
});
