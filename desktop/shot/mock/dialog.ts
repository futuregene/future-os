/** Mock of `@tauri-apps/plugin-dialog` — canned paths, no native dialog. */
// The UI test harness assigns a per-case directory before load
// (`harness/lib/run.mjs`), so a workspace form picks a path that exists on this
// machine and each case gets its own.
function harnessPick(kind: "directory" | "file"): string | null {
  const picked = (globalThis as unknown as { __harnessPickPaths?: Record<string, string> }).__harnessPickPaths;
  return picked?.[kind] ?? null;
}

export async function open(options?: { directory?: boolean; multiple?: boolean }) {
  const path = harnessPick(options?.directory ? "directory" : "file")
    ?? (options?.directory
      ? "/Users/lixin/Research/dopamine-decision"
      : "/Users/lixin/Research/dopamine-decision/papers/frank-2024.pdf");
  return options?.multiple ? [path] : path;
}

export async function save(options?: { defaultPath?: string }) {
  return options?.defaultPath ?? "/Users/lixin/Research/dopamine-decision/notes/compare.md";
}

export async function message(): Promise<void> {}

export async function ask(): Promise<boolean> {
  return true;
}

export async function confirm(): Promise<boolean> {
  return true;
}
