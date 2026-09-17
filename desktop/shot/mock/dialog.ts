/** Mock of `@tauri-apps/plugin-dialog` — canned paths, no native dialog. */
export async function open(options?: { directory?: boolean; multiple?: boolean }) {
  const path = options?.directory
    ? "/Users/lixin/Research/dopamine-decision"
    : "/Users/lixin/Research/dopamine-decision/papers/frank-2024.pdf";
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
