import { isRecord } from "./utils";

/** Process facts owned by the host, matching future_rpc::shell_result. */
export type ShellStatus = "exited" | "launch_failed" | "execution_failed" | "timed_out" | "cancelled" | "not_started";
export interface ShellAttempt {
  status: ShellStatus;
  exit_code: number | null;
  duration_ms: number;
  output: string;
  output_truncated: boolean;
  escalated: boolean;
}
export interface ShellResult {
  command: string;
  cwd: string;
  duration_ms: number;
  status: ShellStatus;
  exit_code: number | null;
  is_soft_fail: boolean;
  is_error: boolean;
  attempts: ShellAttempt[];
  approval: string | null;
  note: string | null;
}

const statuses = new Set(["exited", "launch_failed", "execution_failed", "timed_out", "cancelled", "not_started"]);
const nullableString = (value: unknown) => value === null || typeof value === "string";
const duration = (value: unknown) => typeof value === "number" && Number.isFinite(value) && value >= 0;
const exitCode = (value: unknown) => value === null || Number.isInteger(value);

/** Validate the envelope; never derive status from output or shell syntax. */
export function shellResult(value: unknown): ShellResult | undefined {
  if (!isRecord(value) || typeof value.command !== "string" || typeof value.cwd !== "string"
    || !duration(value.duration_ms) || typeof value.status !== "string" || !statuses.has(value.status)
    || !exitCode(value.exit_code) || typeof value.is_soft_fail !== "boolean" || typeof value.is_error !== "boolean"
    || !nullableString(value.approval) || !nullableString(value.note)
    || !Array.isArray(value.attempts) || value.attempts.length > 2
    || !value.attempts.every(attempt => isRecord(attempt) && typeof attempt.status === "string" && statuses.has(attempt.status)
      && exitCode(attempt.exit_code) && duration(attempt.duration_ms) && typeof attempt.output === "string"
      && typeof attempt.output_truncated === "boolean" && typeof attempt.escalated === "boolean"))
    return undefined;
  return value as unknown as ShellResult;
}
