import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  clearPendingPrompt,
  loadPendingPrompt,
  savePendingPrompt,
  type PendingPrompt,
} from "../pendingPromptStorage";

const mockData = new Map<string, string>();

jest.mock("@react-native-async-storage/async-storage", () => ({
  __esModule: true,
  default: {
    getItem: jest.fn(async (key: string) => mockData.get(key) ?? null),
    setItem: jest.fn(async (key: string, value: string) => {
      mockData.set(key, value);
    }),
    removeItem: jest.fn(async (key: string) => {
      mockData.delete(key);
    }),
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
  beforeEach(() => {
    mockData.clear();
    jest.clearAllMocks();
  });

  test("round-trips a staged prompt", async () => {
    await savePendingPrompt(pending);
    expect(mockedAsync.setItem).toHaveBeenCalledWith(
      "futureos.remote.pending-prompt.v1",
      JSON.stringify(pending),
    );
    mockedAsync.getItem.mockResolvedValueOnce(JSON.stringify(pending));
    await expect(loadPendingPrompt()).resolves.toEqual(pending);
  });

  test("ignores a record with an invalid shape", async () => {
    mockedAsync.getItem.mockResolvedValueOnce(JSON.stringify({ version: 1 }));
    await expect(loadPendingPrompt()).resolves.toBeNull();
  });

  test("ignores a record with corrupt JSON", async () => {
    mockedAsync.getItem.mockResolvedValueOnce("not json{");
    await expect(loadPendingPrompt()).resolves.toBeNull();
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

  test("a matching clear cannot delete a newer prompt saved concurrently", async () => {
    await savePendingPrompt(pending);
    let releaseRead!: () => void;
    const readStarted = new Promise<void>(resolve => {
      mockedAsync.getItem.mockImplementationOnce(async () => {
        resolve();
        await new Promise<void>(release => {
          releaseRead = release;
        });
        return JSON.stringify(pending);
      });
    });

    const clearing = clearPendingPrompt(pending.commandId);
    await readStarted;
    const newer = { ...pending, commandId: "prompt_2", text: "newer" };
    const saving = savePendingPrompt(newer);

    expect(mockedAsync.setItem).toHaveBeenCalledTimes(1);
    releaseRead();
    await Promise.all([clearing, saving]);
    await expect(loadPendingPrompt()).resolves.toEqual(newer);
  });
});
