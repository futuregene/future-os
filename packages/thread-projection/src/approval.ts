/**
 * Approval-request semantic model: the typed wire `action` shape and the
 * field-by-field parsers that turn the agent's JSON payloads into it. Pure and
 * security-relevant — malformed backend data must never reach a render as an
 * unchecked value, so both platforms consume the same guarded parsers.
 */

import { isRecord } from "./utils";

/** v2 parsed save_suggestion — the file rule to persist on "allow in this
 * workspace". `path` is a glob (workspace-relative, or ~/absolute). */
export interface ApprovalSaveSuggestion {
  path?: string;
  access?: string; // "read" | "write"
  rules?: Array<{ path: string; access: "read" | "write" }>;
}

/** P2 structured action payload (parsed from `actionPayload` JSON). */
export interface ApprovalAction {
  tool: string;
  category: string;
  behavior?: "manage_files";
  summary?: string;
  command?: string;
  paths?: string[];
  writes?: Array<{ path: string; preview?: string }>;
  deletes?: Array<{ path: string }>;
  // sandbox_escalation: model-provided reason and the file paths the sandbox
  // blocked (extracted from the failed run — no raw stderr dump).
  justification?: string;
  blockedPaths?: string[];
  targets?: Array<{ path: string; scope: "file" | "subtree" }>;
  scope?: {
    cwd: string;
    insideWorkspace: boolean;
    estimatedBlastRadius: "low" | "medium" | "high";
  };
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === "string")
  );
}

function isPathEntryArray(
  value: unknown,
): value is Array<{ path: string; preview?: string }> {
  return (
    Array.isArray(value) &&
    value.every(
      (item) =>
        isRecord(item) &&
        typeof item.path === "string" &&
        (item.preview === undefined || typeof item.preview === "string"),
    )
  );
}

function isCapabilityTargetArray(
  value: unknown,
): value is NonNullable<ApprovalAction["targets"]> {
  return (
    Array.isArray(value) &&
    value.length > 0 &&
    value.length <= 8 &&
    value.every(
      (item) =>
        isRecord(item) &&
        typeof item.path === "string" &&
        item.path.length > 0 &&
        (item.scope === "file" || item.scope === "subtree"),
    )
  );
}

function isScope(
  value: unknown,
): value is NonNullable<ApprovalAction["scope"]> {
  return (
    isRecord(value) &&
    typeof value.cwd === "string" &&
    typeof value.insideWorkspace === "boolean" &&
    (value.estimatedBlastRadius === "low" ||
      value.estimatedBlastRadius === "medium" ||
      value.estimatedBlastRadius === "high")
  );
}

/**
 * Parse the P2 structured payloads field-by-field rather than asserting the
 * whole shape: required scalars are validated, and each optional field the UI
 * iterates is dropped unless it has the expected shape, so malformed backend
 * data can never reach the render as an unchecked value.
 */
export function parseAction(payload: unknown): ApprovalAction | null {
  if (!payload) return null;
  let parsed: unknown;
  if (typeof payload === "string") {
    try {
      parsed = JSON.parse(payload);
    } catch {
      return null;
    }
  } else {
    parsed = payload;
  }
  if (
    !isRecord(parsed) ||
    typeof parsed.tool !== "string" ||
    typeof parsed.category !== "string"
  )
    return null;
  const capabilityTargets = isCapabilityTargetArray(parsed.targets)
    ? parsed.targets
    : undefined;
  if (
    parsed.category === "windows_write_capability" &&
    (parsed.behavior !== "manage_files" || !capabilityTargets)
  ) {
    return null;
  }
  return {
    behavior: parsed.behavior === "manage_files" ? parsed.behavior : undefined,
    blockedPaths: isStringArray(parsed.blocked_paths)
      ? parsed.blocked_paths
      : undefined,
    category: parsed.category,
    command: typeof parsed.command === "string" ? parsed.command : undefined,
    deletes: isPathEntryArray(parsed.deletes) ? parsed.deletes : undefined,
    justification:
      typeof parsed.justification === "string" &&
      parsed.justification.length > 0
        ? parsed.justification
        : undefined,
    paths: isStringArray(parsed.paths) ? parsed.paths : undefined,
    scope: isScope(parsed.scope) ? parsed.scope : undefined,
    summary: typeof parsed.summary === "string" ? parsed.summary : undefined,
    targets: capabilityTargets,
    tool: parsed.tool,
    writes: isPathEntryArray(parsed.writes) ? parsed.writes : undefined,
  };
}

export function parseSaveSuggestion(
  payload: string | null | undefined,
): ApprovalSaveSuggestion | null {
  if (!payload) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload);
  } catch {
    return null;
  }
  if (!isRecord(parsed)) {
    return null;
  }
  if (typeof parsed.path === "string" && typeof parsed.access === "string") {
    return { access: parsed.access, path: parsed.path };
  }
  if (
    Array.isArray(parsed.rules) &&
    parsed.rules.length > 0 &&
    parsed.rules.length <= 8 &&
    parsed.rules.every(
      (rule) =>
        isRecord(rule) &&
        typeof rule.path === "string" &&
        rule.path.length > 0 &&
        (rule.access === "read" || rule.access === "write"),
    )
  ) {
    return {
      rules: parsed.rules as Array<{ path: string; access: "read" | "write" }>,
    };
  }
  return null;
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
    if (typeof current !== "string") return current;
    current = JSON.parse(current) as unknown;
  }
  return current;
}

export function formatRequestedAction(
  action: string | null | undefined,
): string {
  if (!action) return "";

  try {
    const parsed = unwrapNestedJson(action);
    if (isRecord(parsed) && typeof parsed.command === "string") {
      return parsed.command;
    }
    return JSON.stringify(parsed, null, 2);
  } catch {
    return action;
  }
}

/** Guard for the wire `action` blob on an approval payload — tolerates a JSON
 * string (how some bridges serialize it) as well as an already-parsed object. */
function asRecord(value: unknown): Record<string, unknown> | null {
  if (typeof value === "string") {
    try {
      value = JSON.parse(value);
    } catch {
      return null;
    }
  }
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : null;
}

function pathEntries(action: Record<string, unknown>, key: string): string[] {
  const entries = Array.isArray(action[key]) ? action[key] : [];
  return entries
    .map((entry) => asRecord(entry)?.path as unknown as string)
    .filter(
      (path): path is string => typeof path === "string" && path.length > 0,
    );
}

/**
 * The file path(s) an approval would write/read — surfaced so the user can judge
 * the request. Reads the wire `action` (writes[].path, then paths[]).
 */
export function approvalPaths(payload: { action?: unknown }): string[] {
  const action = asRecord(payload.action);
  if (!action) return [];
  const fromWrites = pathEntries(action, "writes");
  if (fromWrites.length > 0) return fromWrites;
  const fromTargets = pathEntries(action, "targets");
  if (fromTargets.length > 0) return fromTargets;
  const paths = Array.isArray(action.paths) ? action.paths : [];
  return paths.filter(
    (path): path is string => typeof path === "string" && path.length > 0,
  );
}

/** Paths an approval would delete (action.deletes) — a delete request is judged
 * by what it removes, so deletes render independently from writes/paths. */
export function approvalDeletes(payload: { action?: unknown }): string[] {
  const action = asRecord(payload.action);
  if (!action) return [];
  return pathEntries(action, "deletes");
}

/** The command a shell/escalation approval would run (action.command), or null. */
export function approvalCommand(payload: { action?: unknown }): string | null {
  const action = asRecord(payload.action);
  const command =
    action && typeof action.command === "string" ? action.command : null;
  return command && command.length > 0 ? command : null;
}
