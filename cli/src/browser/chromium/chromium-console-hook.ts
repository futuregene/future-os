/**
 * Console hook manager for Chromium CDP.
 *
 * All methods accept the CdpSession explicitly — never assume
 * the session passed at construction time is the right one.
 * The session depends on the page being operated on.
 */
import type { CdpSession } from "./cdp-connection.js";
import { CONSOLE_HOOK_INVOCATION_SOURCE } from "../scripts/console-hook-script.js";

/**
 * Install console hook on the current document of a page session.
 * Safe to call multiple times — the hook script is idempotent.
 */
export async function installConsoleHook(session: CdpSession): Promise<void> {
  await session.send("Runtime.evaluate", {
    expression: CONSOLE_HOOK_INVOCATION_SOURCE,
  }).catch(() => undefined);
}

/**
 * Read buffered console logs from a page session.
 */
export async function readConsoleLogs(
  session: CdpSession,
  level?: string,
): Promise<{
  logs: Array<{ level: string; text: string; time: string }>;
  note?: string;
}> {
  const raw = await session.send("Runtime.evaluate", {
    expression: "(globalThis.__futureConsoleLogs) || []",
    returnByValue: true,
  }) as { result: { value: unknown } };

  const logs = Array.isArray(raw?.result?.value)
    ? raw.result.value as Array<Record<string, unknown>>
    : [];

  const filtered = logs
    .filter(e => !level || e.level === level)
    .map(e => ({
      level: String(e.level ?? ""),
      text: String(e.text ?? ""),
      time: String(e.time ?? ""),
    }));

  return {
    logs: filtered,
    note: filtered.length === 0
      ? "No buffered console messages. The hook captures messages after a Future browser tool has touched the page."
      : undefined,
  };
}

/**
 * Wrap an action with a temporary preload script.
 *
 * 1. Page.addScriptToEvaluateOnNewDocument → get identifier
 * 2. Execute the action
 * 3. Page.removeScriptToEvaluateOnNewDocument(identifier)
 */
export async function withTemporaryPreload<T>(
  session: CdpSession,
  action: () => Promise<T>,
): Promise<T> {
  const result = await session.send("Page.addScriptToEvaluateOnNewDocument", {
    source: CONSOLE_HOOK_INVOCATION_SOURCE,
  }) as { identifier: string };

  try {
    return await action();
  } finally {
    await session.send("Page.removeScriptToEvaluateOnNewDocument", {
      identifier: result.identifier,
    }).catch(() => undefined);
  }
}
