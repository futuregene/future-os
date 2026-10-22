import AsyncStorage from "@react-native-async-storage/async-storage";

/**
 * The task form's unfinished draft, kept across a cold start.
 *
 * The editor is a long form (name, prompt, model, thinking level, trigger,
 * timezone…), and a phone leaves the app for a minute-long background read at a
 * moment's notice. A draft that only lived in component state came back empty —
 * the user was shown the same blank "new task" page they had been filling in.
 * So the draft is written here as it changes and read back when the form opens.
 *
 * It is keyed by the paired desktop *and* the task, because a task id means
 * nothing on another machine, and by the prompt version of an edited task so a
 * draft never masks a prompt somebody else has since changed.
 */
const DRAFT_VERSION = 1;
const KEY_PREFIX = "futureos.tasks.draft.v1:";

export interface StoredTaskDraft {
  version: number;
  /**
   * The task's prompt version when the draft was written, so an edit draft can
   * be dropped once the task's prompt moved underneath it. Null for a new task,
   * which has no version to compare against.
   */
  promptVersion: number | null;
  savedAt: number;
  /** The editor's own draft shape; validated by the caller after a read. */
  draft: unknown;
}

/** The storage slot for one form: `taskId` null is the "new task" form. */
export function taskDraftKey(desktopId: string, taskId: string | null): string {
  return `${KEY_PREFIX}${desktopId}:${taskId ?? "create"}`;
}

export async function saveTaskDraft(
  key: string,
  draft: unknown,
  promptVersion: number | null,
): Promise<void> {
  const record: StoredTaskDraft = {
    version: DRAFT_VERSION,
    promptVersion,
    savedAt: Date.now(),
    draft,
  };
  try {
    await AsyncStorage.setItem(key, JSON.stringify(record));
  } catch {
    // A full disk must not break typing into the form: the draft is a
    // convenience, and the in-memory one is still there.
  }
}

export async function loadTaskDraft(key: string): Promise<StoredTaskDraft | null> {
  let raw: string | null;
  try {
    raw = await AsyncStorage.getItem(key);
  } catch {
    return null;
  }
  if (!raw) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    // A truncated write is discarded, never migrated.
    return null;
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
  const record = parsed as Partial<StoredTaskDraft>;
  if (record.version !== DRAFT_VERSION) return null;
  if (!record.draft || typeof record.draft !== "object" || Array.isArray(record.draft))
    return null;
  return {
    version: DRAFT_VERSION,
    promptVersion: typeof record.promptVersion === "number" ? record.promptVersion : null,
    savedAt: typeof record.savedAt === "number" ? record.savedAt : 0,
    draft: record.draft,
  };
}

export async function clearTaskDraft(key: string): Promise<void> {
  try {
    await AsyncStorage.removeItem(key);
  } catch {
    // Nothing to undo: the next write replaces it and the next save clears it.
  }
}
