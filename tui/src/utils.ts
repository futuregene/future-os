/**
 * Text processing utilities for terminal rendering.
 * Ported from pi's utils.ts — grapheme-aware width, ANSI code tracking,
 * word wrapping, truncation, and overlay compositing.
 */

// ─── Grapheme Width ────────────────────────────────────────────────────────

const REGIONAL_INDICATOR = /^\p{RI}{2}/u;

function isRegionalIndicatorPair(s: string): boolean {
  return REGIONAL_INDICATOR.test(s);
}

function isKeycapSequence(s: string): boolean {
  if (s.length < 2) return false;
  const code = s.codePointAt(0);
  if (code === undefined) return false;
  // Keycap: digit/#/* + U+FE0F + U+20E3
  if (code === 0x23 || code === 0x2A || (code >= 0x30 && code <= 0x39)) {
    return s.includes("⃣");
  }
  return false;
}

function isEmojiFlag(s: string): boolean {
  // Regional indicator pairs
  if (isRegionalIndicatorPair(s)) return true;
  // Flag emoji tag sequences: U+1F3F4 + tags + U+E007F
  if (s.codePointAt(0) === 0x1F3F4) return true;
  return false;
}

function isEmojiModifierBase(code: number): boolean {
  return (
    (code >= 0x261D && code <= 0x270F) || // misc symbols
    code === 0x1F385 || // Santa
    code === 0x1F3C3 || code === 0x1F3C4 || // runner, surfer
    (code >= 0x1F3CA && code <= 0x1F3CC) || // swimmer, etc
    code === 0x1F3F3 || // flag
    (code >= 0x1F441 && code <= 0x1F444) || // eyes, mouth
    (code >= 0x1F446 && code <= 0x1F450) || // hands
    (code >= 0x1F466 && code <= 0x1F478) || // people
    code === 0x1F481 || code === 0x1F482 || // guards
    (code >= 0x1F483 && code <= 0x1F487) || // various people
    code === 0x1F48F || code === 0x1F491 || // kiss, couple
    (code >= 0x1F574 && code <= 0x1F590) || // various
    code === 0x1F595 || code === 0x1F596 || // middle finger, vulcan
    (code >= 0x1F645 && code <= 0x1F647) || // gestures
    (code >= 0x1F64B && code <= 0x1F64F) || // gestures
    (code >= 0x1F6A3 && code <= 0x1F6B6) || // rowboat, pedestrian
    code === 0x1F6C0 || // bath
    code === 0x1F918 || code === 0x1F919 || // handshake variants
    (code >= 0x1F91D && code <= 0x1F91F) || // handshake, etc
    (code >= 0x1F926 && code <= 0x1F935) || // various
    (code >= 0x1F9B5 && code <= 0x1F9B9) || // leg, foot, etc
    (code >= 0x1F9BB && code <= 0x1F9BF) || // various
    code === 0x1F9D1 || code === 0x1F9D2 || code === 0x1F9D3 // people
  );
}

function isEmojiModifier(code: number): boolean {
  return code >= 0x1F3FB && code <= 0x1F3FF; // skin tones
}

function isExtendedPictographic(code: number): boolean {
  return (
    (code >= 0x1F300 && code <= 0x1F64F) || // misc symbols, emoticons
    (code >= 0x1F680 && code <= 0x1F6FF) || // transport
    (code >= 0x1F900 && code <= 0x1F9FF) || // supplemental
    (code >= 0x1FA00 && code <= 0x1FA6F) || // chess, etc
    (code >= 0x1FA70 && code <= 0x1FAFF) || // symbols ext-a
    code === 0x00A9 || code === 0x00AE || // copyright, registered
    code === 0x203C || code === 0x2049 || // double !!, !?
    code === 0x2122 || code === 0x2139 || // tm, i
    code === 0x2194 || code === 0x2199 || code === 0x21A9 || code === 0x21AA || // arrows
    (code >= 0x231A && code <= 0x2328) || // watch, hourglass, keyboard
    code === 0x23CF || // eject
    (code >= 0x23E9 && code <= 0x23F3) || // media controls
    (code >= 0x23F8 && code <= 0x23FA) || // media controls
    code === 0x24C2 || // circled M
    (code >= 0x25AA && code <= 0x25AB) || // squares
    code === 0x25B6 || code === 0x25C0 || // triangles
    (code >= 0x25FB && code <= 0x25FE) || // squares
    (code >= 0x2600 && code <= 0x27BF) || // many symbols
    (code >= 0x2934 && code <= 0x2935) || // arrows
    (code >= 0x2B05 && code <= 0x2B07) || // arrows
    (code >= 0x2B1B && code <= 0x2B1C) || // squares
    code === 0x2B50 || code === 0x2B55 || // star, circle
    code === 0x3030 || code === 0x303D || // wavy dash, part alternation
    code === 0x3297 || code === 0x3299 // circled marks
  );
}

function hasEmojiPresentation(s: string): boolean {
  // Check for variation selector-16 (U+FE0F) making it emoji
  return s.includes("️");
}

function isCJK(code: number): boolean {
  return (
    (code >= 0x1100 && code <= 0x115F) || // Hangul Jamo
    (code >= 0x2329 && code <= 0x232A) || // angle brackets
    (code >= 0x2E80 && code <= 0x303E) || // CJK radicals
    (code >= 0x3040 && code <= 0x33BF) || // Hiragana, Katakana, Bopomofo, Hangul, CJK compat
    (code >= 0x3400 && code <= 0x4DBF) || // CJK ext A
    (code >= 0x4E00 && code <= 0xA4CF) || // CJK unified
    (code >= 0xA960 && code <= 0xA97C) || // Hangul extended
    (code >= 0xAC00 && code <= 0xD7A3) || // Hangul syllables
    (code >= 0xF900 && code <= 0xFAFF) || // CJK compat ideographs
    (code >= 0xFE10 && code <= 0xFE19) || // vertical forms
    (code >= 0xFE30 && code <= 0xFE6F) || // CJK compat forms
    (code >= 0xFF01 && code <= 0xFF60) || // fullwidth forms
    (code >= 0xFFE0 && code <= 0xFFE6) || // fullwidth signs
    (code >= 0x1B000 && code <= 0x1B2FF) || // Kana supplement/extended
    (code >= 0x1F200 && code <= 0x1F2FF) || // enclosed ideographic
    (code >= 0x20000 && code <= 0x2FFFF) || // CJK ext B+
    (code >= 0x30000 && code <= 0x3FFFF)   // CJK ext G+
  );
}

function isZeroWidth(code: number): boolean {
  return (
    code === 0x200B || // zero-width space
    code === 0x200C || // zero-width non-joiner
    code === 0x200D || // zero-width joiner
    code === 0xFEFF || // BOM / ZWNBSP
    code === 0x200E || // left-to-right mark
    code === 0x200F || // right-to-left mark
    code === 0x061C || // ALM
    code === 0x2060 || // word joiner
    code === 0x2061 || code === 0x2062 || code === 0x2063 || code === 0x2064 || // invisible ops
    (code >= 0x0300 && code <= 0x036F) || // combining diacritical marks
    (code >= 0x0483 && code <= 0x0489) || // combining cyrillic
    (code >= 0x0591 && code <= 0x05BD) || // combining Hebrew
    (code >= 0x0610 && code <= 0x061A) || // combining Arabic
    (code >= 0x064B && code <= 0x065F) || // Arabic
    code === 0x0670 || // Arabic
    (code >= 0x06D6 && code <= 0x06DC) || // Arabic
    (code >= 0x06DF && code <= 0x06E4) || // Arabic
    (code >= 0x06E7 && code <= 0x06E8) || // Arabic
    (code >= 0x06EA && code <= 0x06ED) || // Arabic
    code === 0x0711 || // Syriac
    (code >= 0x0730 && code <= 0x074A) || // Syriac
    (code >= 0x07A6 && code <= 0x07B0) || // Thaana
    (code >= 0x0900 && code <= 0x0902) || // Devanagari
    code === 0x093A || code === 0x093C || // Devanagari
    (code >= 0x0941 && code <= 0x0948) || // Devanagari
    code === 0x094D || code === 0x0951 || code === 0x0955 || code === 0x0962 || code === 0x0963 ||
    (code >= 0x1DC0 && code <= 0x1DFF) || // combining diacritical marks supplement
    (code >= 0x20D0 && code <= 0x20FF) || // combining diacritical marks for symbols
    (code >= 0xFE20 && code <= 0xFE2F)   // combining half marks
  );
}

const graphemeWidthCache = new Map<number, number>();

export function graphemeWidth(grapheme: string): number {
  if (grapheme.length === 0) return 0;
  const code = grapheme.codePointAt(0);
  if (code === undefined) return 0;

  const cached = graphemeWidthCache.get(code);
  if (cached !== undefined) return cached;

  let width: number;
  if (isZeroWidth(code)) {
    width = 0;
  } else if (isEmojiFlag(grapheme) || hasEmojiPresentation(grapheme) ||
             (isExtendedPictographic(code) && !grapheme.includes("︎")) ||
             isKeycapSequence(grapheme)) {
    width = 2;
  } else if (isCJK(code)) {
    width = 2;
  } else if (code >= 0x1F000) {
    // High-plane characters default to 2
    width = 2;
  } else {
    width = 1;
  }

  graphemeWidthCache.set(code, width);
  return width;
}

// ─── Visible Width ─────────────────────────────────────────────────────────

const segmenter = new Intl.Segmenter("en", { granularity: "grapheme" });
const visibleWidthCache = new Map<string, number>();
const VISIBLE_WIDTH_CACHE_MAX = 2000;

export function visibleWidth(s: string): number {
  const cached = visibleWidthCache.get(s);
  if (cached !== undefined) return cached;

  let width = 0;
  for (const { segment } of segmenter.segment(s)) {
    width += graphemeWidth(segment);
  }

  if (visibleWidthCache.size < VISIBLE_WIDTH_CACHE_MAX) {
    visibleWidthCache.set(s, width);
  }
  return width;
}

export function clearVisibleWidthCache(): void {
  visibleWidthCache.clear();
  graphemeWidthCache.clear();
}

// ─── Tab Handling ──────────────────────────────────────────────────────────

export function replaceTabs(s: string, tabWidth = 3): string {
  return s.replace(/\t/g, " ".repeat(tabWidth));
}

// ─── Normalize Terminal Output ─────────────────────────────────────────────

// Thai/Lao AM (above-main) combining vowels — only the standalone form
// (U+0E33, U+0EB3) needs decomposition; all other combining marks are zero-width
// and handled correctly by graphemeWidth/visibleWidth.
const THAI_LAO_AM_REGEX = /[\u0e33\u0eb3]/;
const THAI_LAO_AM_GLOBAL_REGEX = /[\u0e33\u0eb3]/g;

export function normalizeTerminalOutput(s: string): string {
  s = replaceTabs(s);
  // Decompose Thai/Lao AM vowels into base + combining marks for
  // terminal compatibility (matching pi's NFD-based approach).
  if (!THAI_LAO_AM_REGEX.test(s)) return s;
  return s.replace(THAI_LAO_AM_GLOBAL_REGEX, (char) =>
    char === "\u0e33" ? "\u0e4d\u0e32" : "\u0ecd\u0eb2"
  );
}

// ─── ANSI Code Extraction ──────────────────────────────────────────────────

export interface AnsiCodeResult {
  length: number;
  code: string;
  type: "sgr" | "csi" | "osc" | "apc" | "other";
}

export function extractAnsiCode(s: string, pos: number): AnsiCodeResult | null {
  if (pos >= s.length) return null;
  if (s[pos] !== "\x1b") return null;

  const rest = s.slice(pos);
  if (rest.length < 2) return null;

  // OSC: ESC ]
  if (rest[1] === "]") {
    // Terminated by BEL (\x07) or ESC \ (ST)
    const belIdx = rest.indexOf("\x07", 2);
    const stIdx = rest.indexOf("\x1b\\", 2);
    let endIdx: number;
    if (belIdx !== -1 && stIdx !== -1) endIdx = Math.min(belIdx, stIdx);
    else if (belIdx !== -1) endIdx = belIdx;
    else if (stIdx !== -1) endIdx = stIdx;
    else return null; // unterminated

    return {
      length: endIdx + (rest[endIdx] === "\x07" ? 1 : 2),
      code: rest.slice(0, endIdx + (rest[endIdx] === "\x07" ? 1 : 2)),
      type: "osc",
    };
  }

  // APC: ESC _
  if (rest[1] === "_") {
    const stIdx = rest.indexOf("\x1b\\", 2);
    if (stIdx === -1) return null;
    return {
      length: stIdx + 2,
      code: rest.slice(0, stIdx + 2),
      type: "apc",
    };
  }

  // CSI: ESC [
  if (rest[1] === "[") {
    let i = 2;
    while (i < rest.length) {
      const c = rest.charCodeAt(i);
      if (c >= 0x40 && c <= 0x7E) {
        const code = rest.slice(0, i + 1);
        return {
          length: i + 1,
          code,
          type: code.endsWith("m") ? "sgr" : "csi",
        };
      }
      if (c < 0x20 || c > 0x3F) break; // not valid CSI parameter
      i++;
    }
    return null; // unterminated
  }

  // SS3: ESC O
  if (rest[1] === "O" && rest.length >= 3) {
    return { length: 3, code: rest.slice(0, 3), type: "csi" };
  }

  // Meta/Alt: ESC + single char
  if (rest.length >= 2) {
    return { length: 2, code: rest.slice(0, 2), type: "other" };
  }

  return null;
}

// ─── ANSI Code Tracker ─────────────────────────────────────────────────────

export interface AnsiState {
  bold: boolean;
  dim: boolean;
  italic: boolean;
  underline: boolean;
  blink: boolean;
  inverse: boolean;
  hidden: boolean;
  strikethrough: boolean;
  fg: string | null;   // ANSI color sequence, e.g. "38;5;45"
  bg: string | null;
  link: string | null;  // OSC 8 hyperlink URL
}

function emptyAnsiState(): AnsiState {
  return {
    bold: false, dim: false, italic: false, underline: false,
    blink: false, inverse: false, hidden: false, strikethrough: false,
    fg: null, bg: null, link: null,
  };
}

export class AnsiCodeTracker {
  private state: AnsiState = emptyAnsiState();
  private stack: AnsiState[] = [];

  reset(): void {
    this.state = emptyAnsiState();
    this.stack = [];
  }

  pushState(): void {
    this.stack.push({ ...this.state });
  }

  popState(): void {
    const prev = this.stack.pop();
    if (prev) this.state = prev;
  }

  feed(code: string): void {
    if (code === "\x1b[0m" || code === "\x1b[m") {
      this.state = emptyAnsiState();
      return;
    }

    const sgrMatch = code.match(/^\x1b\[([\d;]*)m$/);
    if (!sgrMatch) {
      // OSC 8 hyperlink
      const oscMatch = code.match(/^\x1b\]8;([^;\x07]*);([^\x07]*)\x07$/);
      if (oscMatch) {
        const url = oscMatch[2] || null;
        this.state.link = url;
      }
      return;
    }

    const params = sgrMatch[1].split(";").map(Number);
    for (let i = 0; i < params.length; i++) {
      const p = params[i];
      switch (p) {
        case 0: this.state = emptyAnsiState(); break;
        case 1: this.state.bold = true; break;
        case 2: this.state.dim = true; break;
        case 3: this.state.italic = true; break;
        case 4: this.state.underline = true; break;
        case 5: case 6: this.state.blink = true; break;
        case 7: this.state.inverse = true; break;
        case 8: this.state.hidden = true; break;
        case 9: this.state.strikethrough = true; break;
        case 21: case 22: this.state.bold = false; this.state.dim = false; break;
        case 23: this.state.italic = false; break;
        case 24: this.state.underline = false; break;
        case 25: this.state.blink = false; break;
        case 27: this.state.inverse = false; break;
        case 28: this.state.hidden = false; break;
        case 29: this.state.strikethrough = false; break;
        case 38:
          if (params[i + 1] === 5 && params[i + 2] !== undefined) {
            this.state.fg = `38;5;${params[i + 2]}`;
            i += 2;
          } else if (params[i + 1] === 2 && params[i + 4] !== undefined) {
            this.state.fg = `38;2;${params[i + 2]};${params[i + 3]};${params[i + 4]}`;
            i += 4;
          }
          break;
        case 48:
          if (params[i + 1] === 5 && params[i + 2] !== undefined) {
            this.state.bg = `48;5;${params[i + 2]}`;
            i += 2;
          } else if (params[i + 1] === 2 && params[i + 4] !== undefined) {
            this.state.bg = `48;2;${params[i + 2]};${params[i + 3]};${params[i + 4]}`;
            i += 4;
          }
          break;
        case 39: this.state.fg = null; break;
        case 49: this.state.bg = null; break;
        // 256-color fg: 38;5;N handled above
        // 256-color bg: 48;5;N handled above
      }
    }
  }

  getAnsiCode(): string {
    const parts: string[] = [];
    if (this.state.bold) parts.push("1");
    if (this.state.dim) parts.push("2");
    if (this.state.italic) parts.push("3");
    if (this.state.underline) parts.push("4");
    if (this.state.blink) parts.push("5");
    if (this.state.inverse) parts.push("7");
    if (this.state.hidden) parts.push("8");
    if (this.state.strikethrough) parts.push("9");
    if (this.state.fg) parts.push(this.state.fg);
    if (this.state.bg) parts.push(this.state.bg);
    if (parts.length === 0) return "\x1b[0m";
    return `\x1b[${parts.join(";")}m`;
  }

  getState(): Readonly<AnsiState> {
    return this.state;
  }

  /** Get OSC 8 hyperlink open sequence if a link is active. */
  getOsc8Link(): string {
    if (this.state.link) {
      return `\x1b]8;id=xihu;${this.state.link}\x07`;
    }
    return "";
  }

  /** Get OSC 8 close sequence. */
  getOsc8Close(): string {
    return "\x1b]8;;\x07";
  }
}

// ─── Strip ANSI ────────────────────────────────────────────────────────────

export function stripAnsiCodes(s: string): string {
  let result = "";
  let i = 0;
  while (i < s.length) {
    const code = extractAnsiCode(s, i);
    if (code) {
      i += code.length;
    } else {
      result += s[i];
      i++;
    }
  }
  return result;
}

// ─── Word Wrap with ANSI ───────────────────────────────────────────────────

export function wrapTextWithAnsi(text: string, width: number): string[] {
  if (width <= 0) return [];
  const lines: string[] = [];
  const tracker = new AnsiCodeTracker();
  let currentLine = "";
  let currentWidth = 0;
  let i = 0;

  while (i < text.length) {
    if (text[i] === "\n") {
      lines.push(currentLine + finalizeLine(tracker));
      currentLine = "";
      currentWidth = 0;
      i++;
      continue;
    }

    // Check for ANSI code
    const ansi = extractAnsiCode(text, i);
    if (ansi) {
      tracker.feed(ansi.code);
      currentLine += ansi.code;
      i += ansi.length;
      continue;
    }

    // Grab one grapheme
    const segIter = segmenter.segment(text.slice(i))[Symbol.iterator]();
    const segResult = segIter.next();
    if (segResult.done) break;
    const grapheme = segResult.value.segment;
    const gw = graphemeWidth(grapheme);

    if (currentWidth + gw > width) {
      // Word-boundary break: backtrack to last space
      const spaceIdx = currentLine.lastIndexOf(" ");
      if (spaceIdx > 0 && !isAllAnsi(currentLine.slice(0, spaceIdx))) {
        // Check visible width up to space
        const afterSpace = currentLine.slice(spaceIdx + 1);
        // Push line up to space, wrap remainder
        lines.push(currentLine.slice(0, spaceIdx) + finalizeLine(tracker));
        currentLine = tracker.getAnsiCode() + tracker.getOsc8Link() + afterSpace;
        currentWidth = visibleWidth(stripAnsiCodes(afterSpace));
        currentLine += grapheme;
        currentWidth += gw;
      } else {
        // Hard break at width
        lines.push(currentLine + finalizeLine(tracker));
        currentLine = tracker.getAnsiCode() + tracker.getOsc8Link() + grapheme;
        currentWidth = gw;
      }
    } else {
      currentLine += grapheme;
      currentWidth += gw;
    }
    i += grapheme.length;
  }

  if (currentLine.length > 0) {
    lines.push(currentLine + finalizeLine(tracker));
  }

  return lines.length > 0 ? lines : [""];
}

function isAllAnsi(s: string): boolean {
  let i = 0;
  while (i < s.length) {
    const code = extractAnsiCode(s, i);
    if (code) {
      i += code.length;
    } else {
      return false;
    }
  }
  return true;
}

function finalizeLine(tracker: AnsiCodeTracker): string {
  const oscClose = tracker.getState().link ? tracker.getOsc8Close() : "";
  return oscClose + "\x1b[0m";
}

// ─── Apply Background to Line ──────────────────────────────────────────────

export function applyBackgroundToLine(line: string, width: number, bg: number): string {
  const visibleLen = visibleWidth(stripAnsiCodes(line));
  const padding = Math.max(0, width - visibleLen);
  // Reset at end, re-apply styles for padding
  const parts: string[] = [];
  let stylePrefix = "";
  let i = 0;
  while (i < line.length) {
    const code = extractAnsiCode(line, i);
    if (code) {
      stylePrefix += code.code;
      parts.push(code.code);
      i += code.length;
    } else {
      parts.push(line[i]);
      i++;
    }
  }

  const bgCode = `\x1b[48;5;${bg}m`;
  return bgCode + stylePrefix + parts.join("") + "\x1b[0m" + bgCode + " ".repeat(padding) + "\x1b[0m";
}

// ─── Truncate to Width ─────────────────────────────────────────────────────

export interface TruncateOptions {
  ellipsis?: boolean;
  pad?: boolean;
}

export function truncateToWidth(s: string, width: number, opts: TruncateOptions = {}): string {
  if (width <= 0) return "";
  const result = sliceWithWidth(s, width);
  if (result.text.length < s.length && opts.ellipsis) {
    // Replace last char with ellipsis, keeping ANSI context
    return result.text.slice(0, -1) + "…";
  }
  if (opts.pad && result.width < width) {
    return result.text + " ".repeat(width - result.width);
  }
  return result.text;
}

// ─── Slice by Column ───────────────────────────────────────────────────────

export function sliceWithWidth(s: string, maxWidth: number): { text: string; width: number } {
  let result = "";
  let width = 0;
  let i = 0;

  while (i < s.length && width < maxWidth) {
    const code = extractAnsiCode(s, i);
    if (code) {
      result += code.code;
      i += code.length;
      continue;
    }

    const segIter = segmenter.segment(s.slice(i))[Symbol.iterator]();
    const segResult = segIter.next();
    if (segResult.done) break;
    const grapheme = segResult.value.segment;
    const gw = graphemeWidth(grapheme);
    if (width + gw > maxWidth) break;

    result += grapheme;
    width += gw;
    i += grapheme.length;
  }

  return { text: result, width };
}

export function sliceByColumn(s: string, start: number, end?: number): string {
  if (start < 0) start = 0;

  let col = 0;
  let i = 0;
  let result = "";

  // Skip to start
  while (i < s.length && col < start) {
    const code = extractAnsiCode(s, i);
    if (code) {
      i += code.length;
      continue;
    }
    const segIter = segmenter.segment(s.slice(i))[Symbol.iterator]();
    const segResult = segIter.next();
    if (segResult.done) break;
    col += graphemeWidth(segResult.value.segment);
    i += segResult.value.segment.length;
  }

  if (end === undefined) {
    // Return from start to end, including trailing ANSI codes
    let trailing = "";
    let j = i;
    while (j < s.length) {
      const code = extractAnsiCode(s, j);
      if (code) {
        trailing += code.code;
        j += code.length;
      } else {
        trailing = ""; // reset trailing codes before content
        break;
      }
    }
    return s.slice(i);
  }

  // Extract [start, end)
  let ansPrefix = "";
  while (i < s.length && col < end) {
    const code = extractAnsiCode(s, i);
    if (code) {
      ansPrefix += code.code;
      result += code.code;
      i += code.length;
      continue;
    }
    const segIter = segmenter.segment(s.slice(i))[Symbol.iterator]();
    const segResult = segIter.next();
    if (segResult.done) break;
    const gw = graphemeWidth(segResult.value.segment);
    if (col + gw > end) break;
    result += segResult.value.segment;
    col += gw;
    i += segResult.value.segment.length;
  }

  return result;
}

// ─── Extract Segments (for overlay compositing) ────────────────────────────

// Pooled tracker instance for extractSegments (avoids allocation per call)
const pooledStyleTracker = new AnsiCodeTracker();

/**
 * Extract "before" and "after" segments from a line in a single pass.
 * Used for overlay compositing where we need content before and after the overlay region.
 * Preserves styling from before the overlay that should affect content after it.
 */
export function extractSegments(
  line: string,
  beforeEnd: number,
  afterStart: number,
  afterLen: number,
  strictAfter = false,
): { before: string; beforeWidth: number; after: string; afterWidth: number } {
  let before = "",
    beforeWidth = 0,
    after = "",
    afterWidth = 0;
  let currentCol = 0,
    i = 0;
  let pendingAnsiBefore = "";
  let afterStarted = false;
  const afterEnd = afterStart + afterLen;

  pooledStyleTracker.reset();

  while (i < line.length) {
    const ansi = extractAnsiCode(line, i);
    if (ansi) {
      pooledStyleTracker.feed(ansi.code);
      if (currentCol < beforeEnd) {
        pendingAnsiBefore += ansi.code;
      } else if (currentCol >= afterStart && currentCol < afterEnd && afterStarted) {
        after += ansi.code;
      }
      i += ansi.length;
      continue;
    }

    let textEnd = i;
    while (textEnd < line.length && !extractAnsiCode(line, textEnd)) textEnd++;

    for (const { segment } of segmenter.segment(line.slice(i, textEnd))) {
      const w = graphemeWidth(segment);

      if (currentCol < beforeEnd) {
        if (pendingAnsiBefore) {
          before += pendingAnsiBefore;
          pendingAnsiBefore = "";
        }
        before += segment;
        beforeWidth += w;
      } else if (currentCol >= afterStart && currentCol < afterEnd) {
        const fits = !strictAfter || currentCol + w <= afterEnd;
        if (fits) {
          if (!afterStarted) {
            after += pooledStyleTracker.getAnsiCode();
            afterStarted = true;
          }
          after += segment;
          afterWidth += w;
        }
      }

      currentCol += w;
      if (afterLen <= 0 ? currentCol >= beforeEnd : currentCol >= afterEnd) break;
    }
    i = textEnd;
    if (afterLen <= 0 ? currentCol >= beforeEnd : currentCol >= afterEnd) break;
  }

  return { before, beforeWidth, after, afterWidth };
}

