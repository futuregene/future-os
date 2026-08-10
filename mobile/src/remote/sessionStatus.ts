/**
 * Effective run status for a session row — mirrors the desktop sidebar
 * (`ThreadListItem.effectiveRunStatus`): the local run status wins when it
 * reports running/queued, but a session the agent reports as streaming with
 * no local run row (e.g. a prompt started by the TUI/CLI/another machine)
 * still reads as running.
 */
export function effectiveRunStatus(
  status: string | undefined,
  streaming: boolean | undefined,
): string | undefined {
  if (status === "running" || status === "queued") return status;
  if (streaming) return "running";
  return status;
}
