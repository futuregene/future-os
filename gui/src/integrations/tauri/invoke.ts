import type { InvokeArgs, InvokeOptions } from "@tauri-apps/api/core";
import { invoke } from "@tauri-apps/api/core";

/**
 * Centralized, typed wrapper around Tauri's `invoke`.
 *
 * Every frontend call into a `src-tauri` command flows through here so that
 * error shapes are normalized in one place: Tauri rejects with whatever the
 * Rust command returned (commonly a string, but sometimes a structured object),
 * which leaves call sites doing ad-hoc `error instanceof Error ? ... : String(error)`
 * gymnastics. `invokeCommand` always rejects with a real `Error` carrying a
 * readable message.
 *
 * Argument-shape convention (matches the existing `src-tauri` command
 * signatures, which are a fixed contract we cannot change here):
 * - structured input → `{ input }`
 * - single scalar(s) → named key(s), e.g. `{ threadId }`
 */
export async function invokeCommand<T>(
  command: string,
  args?: InvokeArgs,
  options?: InvokeOptions,
): Promise<T> {
  try {
    return await invoke<T>(command, args, options);
  }
  catch (error) {
    throw normalizeInvokeError(command, error);
  }
}

/**
 * Convert an arbitrary value thrown by Tauri into a consistent `Error`.
 */
export function normalizeInvokeError(command: string, error: unknown): Error {
  if (error instanceof Error)
    return error;

  const message = invokeErrorMessage(error);
  return new Error(message || `Tauri command "${command}" failed`);
}

function invokeErrorMessage(error: unknown): string {
  if (typeof error === "string")
    return error;

  if (error == null)
    return "";

  if (typeof error === "object") {
    const record = error as Record<string, unknown>;
    if (typeof record.message === "string")
      return record.message;
    if (typeof record.error === "string")
      return record.error;
    try {
      return JSON.stringify(error);
    }
    catch {
      return String(error);
    }
  }

  return String(error);
}
