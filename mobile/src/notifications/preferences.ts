import AsyncStorage from "@react-native-async-storage/async-storage";

/**
 * Whether this phone raises a reminder when a task finishes.
 *
 * A preference of the *phone*, not of the desktop that ran the task: the
 * reminder is drawn here — a local notification, or the in-app alert that
 * stands in when the OS refuses to show one — so one choice covers every
 * paired machine, and it can be made with no machine connected at all.
 *
 * On by default. A task is the thing the user walked away from, and a reminder
 * they never asked for is worth less than the one they would have missed.
 */
export const DEFAULT_TASK_FINISHED_REMINDERS = true;

const STORAGE_KEY = "future.mobile.taskFinishedReminders";

let enabled = DEFAULT_TASK_FINISHED_REMINDERS;
let initialization: Promise<void> | undefined;
const listeners = new Set<() => void>();

export const getTaskFinishedReminders = (): boolean => enabled;

export function subscribeTaskFinishedReminders(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

function apply(next: boolean): void {
  if (enabled === next) return;
  enabled = next;
  listeners.forEach(listener => listener());
}

/** Read the stored record; anything this app did not write reads as the default. */
export function parseTaskFinishedReminders(value: string | null): boolean {
  if (value === null) return DEFAULT_TASK_FINISHED_REMINDERS;
  try {
    const stored: unknown = JSON.parse(value);
    return typeof stored === "boolean" ? stored : DEFAULT_TASK_FINISHED_REMINDERS;
  } catch {
    // Storage held something that is not our JSON; treat it as absent.
    return DEFAULT_TASK_FINISHED_REMINDERS;
  }
}

/** Restore before the first reminder can be decided on, so "off" holds from launch. */
export function initializeTaskFinishedReminders(): Promise<void> {
  initialization ??= AsyncStorage.getItem(STORAGE_KEY).then(value => {
    apply(parseTaskFinishedReminders(value));
  }).catch(() => {
    // Unavailable storage must not prevent launch; use the default for this run.
    apply(DEFAULT_TASK_FINISHED_REMINDERS);
  });
  return initialization;
}

export async function setTaskFinishedReminders(next: boolean): Promise<void> {
  await initializeTaskFinishedReminders();
  // Commit only after persistence succeeds, keeping the switch honest on failure.
  await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  apply(next);
}
