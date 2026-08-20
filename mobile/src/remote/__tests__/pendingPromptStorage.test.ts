import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  clearPendingPrompt,
  loadPendingPrompt,
  savePendingPrompt,
  type PendingPrompt,
} from "../pendingPromptStorage";

jest.mock("@react-native-async-storage/async-storage", () => ({
  __esModule: true,
  default: {
    getItem: jest.fn(),
    setItem: jest.fn(),
    removeItem: jest.fn(),
  },
}));

const mockedAsync = AsyncStorage as jest.Mocked<typeof AsyncStorage>;
const pending: PendingPrompt = {
  version: 1,
  commandId: "prompt_1",
  draftKey: "s1",
  sessionId: "s1",
  text: "hello",
  attachments: [],
  modelId: "provider/model",
  thinkingLevel: "medium",
  mode: "chat",
  workspaceId: "",
  createdAt: 1,
};

describe("pending prompt storage", () => {
  beforeEach(() => jest.clearAllMocks());

  test("round-trips a staged prompt", async () => {
    await savePendingPrompt(pending);
    expect(mockedAsync.setItem).toHaveBeenCalledWith(
      "futureos.remote.pending-prompt.v1",
      JSON.stringify(pending),
    );
    mockedAsync.getItem.mockResolvedValueOnce(JSON.stringify(pending));
    await expect(loadPendingPrompt()).resolves.toEqual(pending);
  });

  test("a stale completion cannot clear a newer prompt", async () => {
    mockedAsync.getItem.mockResolvedValueOnce(JSON.stringify(pending));
    await clearPendingPrompt("prompt_old");
    expect(mockedAsync.removeItem).not.toHaveBeenCalled();
  });

  test("the matching completion clears its record", async () => {
    mockedAsync.getItem.mockResolvedValueOnce(JSON.stringify(pending));
    await clearPendingPrompt("prompt_1");
    expect(mockedAsync.removeItem).toHaveBeenCalledWith("futureos.remote.pending-prompt.v1");
  });
});
