import type { Root } from "mdast";
import type { Code, Construct, Tokenizer } from "micromark-util-types";
import type { Plugin } from "unified";
import { attention } from "micromark-core-commonmark";

const cjk = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;
const punctuation = /\p{P}/u;
/**
 * Fullwidth/CJK punctuation (`：` `。` `、` `」` `…`, curly quotes). Latin prose
 * writes `:`, so a delimiter run directly after one of these is a CJK-style
 * `**标签：**正文` boundary rather than an ASCII word boundary; intersected with
 * `punctuation`, which also drops the few non-punctuation marks in the ranges.
 */
const cjkPunctuation = /[\u2010-\u201F\u2026\u3001-\u303F\uFF01-\uFF0F\uFF1A-\uFF20\uFF3B-\uFF40\uFF5B-\uFF65]/u;

function matches(pattern: RegExp, code: Code): boolean {
  return code !== null && code > 0 && pattern.test(String.fromCodePoint(code));
}

/**
 * CommonMark treats CJK letters like Latin word characters, so punctuation in
 * `**注意：**正文` prevents the closing delimiter from closing. Allow asterisk
 * emphasis next to punctuation when its outside neighbour is CJK, and let CJK
 * punctuation on the inside close whatever follows it — a label like
 * `**复现：**10 轮对话` is followed by a digit or a Latin name as often as by a
 * Han character, and the ASCII form (`**Warning:**text`) stays literal. Keep the
 * standard tokenizer/resolver for pairing, nesting and the rule of three.
 * This operates on syntax tokens, not rewritten source or decoded text: code,
 * escapes, destinations and streaming source offsets remain untouched.
 */
const tokenize: Tokenizer = function (effects, ok, nok) {
  const context = this;
  const previous = context.previous;
  return attention.tokenize.call(context, effects, code => {
    // The standard tokenizer has just exited and classified this delimiter.
    const token = context.events[context.events.length - 1]?.[1];
    if (token?.type === "attentionSequence") {
      if (matches(cjk, previous) && matches(punctuation, code)) token._open = true;
      if (matches(punctuation, previous) && (matches(cjk, code) || matches(cjkPunctuation, previous))) token._close = true;
    }
    return ok(code);
  }, nok);
};

const cjkAttention: Construct = { ...attention, tokenize };

export const remarkCjkEmphasis: Plugin<[], Root> = function () {
  const data = this.data();
  // Underscores keep CommonMark's intraword restrictions.
  (data.micromarkExtensions ??= []).push({ text: { 42: cjkAttention } });
};
