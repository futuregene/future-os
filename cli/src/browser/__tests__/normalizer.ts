/**
 * Output normalizer for characterization test snapshots.
 * Replaces non-deterministic values with stable placeholders.
 */
import type { BrowserConfig } from "../types.js";

export interface NormalizerContext {
  tempRoot: string;
  endpoint: string;
  pid: number;
  port?: number;
  timestampPattern?: RegExp;
  pageId?: string;
}

export interface NormalizedResult {
  stdout: string;
  stderr: string;
  exitCode: number;
  structuredContent: Record<string, unknown> | null;
  text: string | null;
}

const ISO_TIMESTAMP_RE = /\d{4}-\d{2}-\d{2}T\d{2}[:-]\d{2}[:-]\d{2}(?:\.\d+)?Z?/g;

export function normalizeResult(
  result: NormalizedResult,
  ctx: NormalizerContext,
): NormalizedResult {
  const pidStr = ctx.pid > 0 ? String(ctx.pid) : null;

  return {
    stdout: replaceAllStr(result.stdout, ctx, pidStr),
    stderr: replaceAllStr(result.stderr, ctx, pidStr),
    exitCode: result.exitCode,
    structuredContent: normalizeDeep(result.structuredContent, ctx, pidStr) as Record<string, unknown> | null,
    text: result.text,
  };
}

function replaceAllStr(
  input: string,
  ctx: NormalizerContext,
  pidStr: string | null,
): string {
  let out = input;

  // Specific values first
  out = out.replaceAll(ctx.tempRoot, "<TEMP_ROOT>");
  out = out.replaceAll(ctx.endpoint, "<ENDPOINT>");
  if (pidStr) out = out.replaceAll(pidStr, "<PID>");

  // Normalize any HTTP endpoint on localhost (catches port-differing variants)
  out = out.replace(/http:\/\/127\.0\.0\.1:\d+/g, "<ENDPOINT>");

  // Generic temp directory paths
  out = out.replace(/\/var\/folders\/[^\s"',}]+/g, "<TEMP_DIR>");
  out = out.replace(/\/tmp\/[^\s"',}]+/g, "<TEMP_DIR>");

  // Timestamps
  const tsPattern = ctx.timestampPattern ?? ISO_TIMESTAMP_RE;
  out = out.replace(tsPattern, "<TIMESTAMP>");

  if (ctx.pageId) out = out.replaceAll(ctx.pageId, "<PAGE_ID>");

  return out;
}

function normalizeDeep(
  value: unknown,
  ctx: NormalizerContext,
  pidStr: string | null,
): unknown {
  if (value === null || value === undefined) return value;

  if (typeof value === "string") {
    return replaceAllStr(value, ctx, pidStr);
  }

  if (typeof value === "number") {
    if (pidStr && value === ctx.pid) return "<PID>";
    return value;
  }

  if (Array.isArray(value)) {
    return value.map(v => normalizeDeep(v, ctx, pidStr));
  }

  if (typeof value === "object") {
    const result: Record<string, unknown> = {};
    for (const [key, val] of Object.entries(value as Record<string, unknown>)) {
      result[key] = normalizeDeep(val, ctx, pidStr);
    }
    return result;
  }

  return value;
}

/**
 * Snapshot a config for characterization testing.
 */
export function snapshotConfig(
  config: BrowserConfig,
  _ctx: NormalizerContext,
): Record<string, unknown> {
  const snap = structuredClone(config) as unknown as Record<string, unknown>;
  if (snap.connection && typeof snap.connection === "object") {
    const conn = snap.connection as Record<string, unknown>;
    conn.endpoint = "<ENDPOINT>";
    if (conn.sessionId) conn.sessionId = "<SESSION_ID>";
    if (conn.driverPid !== undefined) conn.driverPid = "<PID>";
  }
  if (snap.activePageId) snap.activePageId = "<PAGE_ID>";
  if (snap.tabOrder) {
    snap.tabOrder = (snap.tabOrder as string[]).map(() => "<PAGE_ID>");
  }
  if (snap.refsPageId) snap.refsPageId = "<PAGE_ID>";
  if (snap.refsUrl) snap.refsUrl = "<NORMALIZED_URL>";
  return snap;
}
