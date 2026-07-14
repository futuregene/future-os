/**
 * Selector resolver: ref→selector resolution, selector parsing, strictness.
 */
import type { BrowserConfig } from "./types.js";

// ── Types ───────────────────────────────────────────────────────────

export type ParsedSelectorEngine = "css" | "xpath" | "text";

export interface ParsedSelector {
  engine: ParsedSelectorEngine;
  body: string;
}

export interface ResolvedTarget {
  /** The original user input. */
  original: string;
  /** How the selector was resolved. */
  source: "ref" | "selector";
  /** The final CSS selector (or xpath/text expression). */
  selector: string;
  /** The ref key, if source is "ref". */
  ref?: string;
  /** Parsed selector engine. */
  parsed: ParsedSelector;
}

// ── Ref → Selector Resolution ───────────────────────────────────────

/**
 * Resolve user input to a concrete selector.
 *
 * Input can be:
 * - A ref (e.g., "b1", "i2", "a3") → looked up in config.refs
 * - A "target" → checked if it's a ref first, then treated as a selector
 * - A direct CSS selector
 */
export function resolveTarget(
  input: string | undefined,
  config: BrowserConfig,
): ResolvedTarget {
  const raw = input?.trim();
  if (!raw) {
    throw new SelectorError("Expected ref, selector, or target.", "missing_input", undefined);
  }

  // If it looks like a ref (e.g., b1, i2, a3) and the config has refs, resolve it
  const refMatch = raw.match(/^[a-z]\d+$/i);

  if (refMatch) {
    const ref = refMatch[0].toLowerCase(); // refs are case-insensitive (a1, A1, etc.)
    if (config.refs?.[ref]) {
      return {
        original: raw,
        source: "ref",
        selector: config.refs[ref],
        ref,
        parsed: parseSelector(config.refs[ref]),
      };
    }
    // It looks like a ref but no matching entry → error with context
    throw new UnknownRefError(ref);
  }

  // It's a direct selector
  return {
    original: raw,
    source: "selector",
    selector: raw,
    parsed: parseSelector(raw),
  };
}

/**
 * Legacy resolver that matches the current selectorFor() behavior:
 * - selector → use directly
 * - target → check if it's a ref, otherwise use as selector
 * - ref → resolve from config.refs
 */
export function legacySelectorFor(
  args: Record<string, unknown>,
  config: BrowserConfig,
): string {
  const selector = typeof args.selector === "string" && args.selector.length > 0
    ? args.selector
    : undefined;
  if (selector) return selector;

  const target = typeof args.target === "string" && args.target.length > 0
    ? args.target
    : undefined;
  const ref = typeof args.ref === "string" && args.ref.length > 0
    ? args.ref
    : (target && /^[a-z]\d+$/i.test(target) ? target : undefined);

  if (ref) {
    const resolved = config.refs?.[ref];
    if (!resolved) throw new UnknownRefError(ref);
    return resolved;
  }

  if (target) return target;

  throw new SelectorError(
    "Expected ref, selector, or target.",
    "missing_input",
    undefined,
  );
}

// ── Selector Parsing ────────────────────────────────────────────────

/**
 * Parse a raw selector string into its engine and body.
 *
 * Supported formats:
 * - Standard CSS: "#id", ".class", "tag", etc.
 * - text=<text> → Playwright text selector
 * - xpath=<expr> → XPath expression
 */
export function parseSelector(raw: string): ParsedSelector {
  if (raw.startsWith("text=")) {
    return { engine: "text", body: raw.slice(5) };
  }
  if (raw.startsWith("xpath=")) {
    return { engine: "xpath", body: raw.slice(6) };
  }
  return { engine: "css", body: raw };
}

// ── Errors ──────────────────────────────────────────────────────────

export class SelectorError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly selector: string | undefined,
  ) {
    super(message);
    this.name = "SelectorError";
  }
}

export class UnknownRefError extends SelectorError {
  constructor(ref: string) {
    super(
      `Unknown browser ref "${ref}". Run browser command snapshot first.`,
      "unknown_ref",
      ref,
    );
    this.name = "UnknownRefError";
  }
}

export class ElementNotFoundError extends SelectorError {
  constructor(selector: string) {
    super(
      `Element not found: "${selector}"`,
      "element_not_found",
      selector,
    );
    this.name = "ElementNotFoundError";
  }
}

export class StrictModeViolationError extends SelectorError {
  constructor(selector: string, count: number) {
    super(
      `Strict mode violation: "${selector}" resolved to ${count} elements. Use a more specific selector or run browser snapshot for unique refs.`,
      "strict_mode_violation",
      selector,
    );
    this.name = "StrictModeViolationError";
  }
}
