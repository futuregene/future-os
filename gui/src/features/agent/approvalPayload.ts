import type { ApprovalAction, ApprovalSaveSuggestion } from "../../integrations/storage/types";
import { isRecord } from "../../lib/objects";

/**
 * Pure parsing/validation for approval-card payloads. Kept out of the
 * ApprovalPrompt view so the security-relevant shape checks (malformed backend
 * data must never reach the render as an unchecked value) are unit-testable.
 */

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(item => typeof item === "string");
}

function isPathEntryArray(value: unknown): value is Array<{ path: string; preview?: string }> {
  return Array.isArray(value) && value.every(item =>
    isRecord(item)
    && typeof item.path === "string"
    && (item.preview === undefined || typeof item.preview === "string"));
}

function isScope(value: unknown): value is NonNullable<ApprovalAction["scope"]> {
  return isRecord(value)
    && typeof value.cwd === "string"
    && typeof value.insideWorkspace === "boolean"
    && (value.estimatedBlastRadius === "low"
      || value.estimatedBlastRadius === "medium"
      || value.estimatedBlastRadius === "high");
}

// Parse the P2 structured payloads field-by-field rather than asserting the
// whole shape: required scalars are validated, and each optional field the UI
// iterates is dropped unless it has the expected shape, so malformed backend
// data can never reach the render as an unchecked value.
export function parseAction(payload: string | null | undefined): ApprovalAction | null {
  if (!payload)
    return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload);
  }
  catch {
    return null;
  }
  if (!isRecord(parsed) || typeof parsed.tool !== "string" || typeof parsed.category !== "string")
    return null;
  return {
    blockedPaths: isStringArray(parsed.blocked_paths) ? parsed.blocked_paths : undefined,
    category: parsed.category,
    command: typeof parsed.command === "string" ? parsed.command : undefined,
    deletes: isPathEntryArray(parsed.deletes) ? parsed.deletes : undefined,
    justification: typeof parsed.justification === "string" && parsed.justification.length > 0
      ? parsed.justification
      : undefined,
    paths: isStringArray(parsed.paths) ? parsed.paths : undefined,
    scope: isScope(parsed.scope) ? parsed.scope : undefined,
    summary: typeof parsed.summary === "string" ? parsed.summary : undefined,
    tool: parsed.tool,
    writes: isPathEntryArray(parsed.writes) ? parsed.writes : undefined,
  };
}

export function parseSaveSuggestion(payload: string | null | undefined): ApprovalSaveSuggestion | null {
  if (!payload)
    return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload);
  }
  catch {
    return null;
  }
  if (
    !isRecord(parsed)
    || typeof parsed.path !== "string"
    || typeof parsed.access !== "string"
  ) {
    return null;
  }
  return { access: parsed.access, path: parsed.path };
}

/**
 * Unwrap a value that may be JSON-encoded up to `maxDepth` times (tool inputs and
 * requested-action payloads arrive double/triple-encoded from the agent),
 * returning the first non-string result. Throws if an intermediate string isn't
 * valid JSON — callers decide whether a non-JSON leaf is an error or the raw
 * value. Shared by the approval card and the continue/retry prompt builder.
 */
export function unwrapNestedJson(value: unknown, maxDepth = 3): unknown {
  let current = value;
  for (let index = 0; index < maxDepth; index += 1) {
    if (typeof current !== "string")
      return current;
    current = JSON.parse(current) as unknown;
  }
  return current;
}

export function formatRequestedAction(action: string | null | undefined): string {
  if (!action)
    return "";

  try {
    const parsed = unwrapNestedJson(action);
    if (isRecord(parsed) && typeof parsed.command === "string") {
      return parsed.command;
    }
    return JSON.stringify(parsed, null, 2);
  }
  catch {
    return action;
  }
}
