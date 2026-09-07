import type { Root } from "mdast";
import type { Extension as FromMarkdownExtension, Handle } from "mdast-util-from-markdown";
import type { Code, Construct, State, Tokenizer } from "micromark-util-types";
import type { Plugin } from "unified";

declare module "micromark-util-types" {
  interface TokenTypeMap {
    latexMathInline: "latexMathInline";
    latexMathDisplay: "latexMathDisplay";
    latexMathMarker: "latexMathMarker";
    latexMathData: "latexMathData";
  }
}

/**
 * Native Markdown syntax for LaTeX's \\(…\\) and standalone \\[…\\] forms.
 * Tokenizing before Markdown escapes are resolved protects TeX from emphasis,
 * links, etc., while leaving code, HTML and link destinations alone. No source
 * rewriting means streaming block offsets still refer to the original text.
 */
export const remarkLatexMath: Plugin<[], Root> = function () {
  const data = this.data();
  (data.micromarkExtensions ??= []).push({
    text: { 92: { name: "latexMathInline", tokenize: tokenizeMath(false) } },
    flow: { 92: { name: "latexMathDisplay", tokenize: tokenizeMath(true), concrete: true } },
  });
  (data.fromMarkdownExtensions ??= []).push(fromMarkdown);
};

const enterMath: Handle = function (token) {
  this.enter(token.type === "latexMathDisplay"
    ? { type: "math", meta: null, value: "" }
    : { type: "inlineMath", value: "" }, token);
  this.buffer();
};

const exitMath: Handle = function (token) {
  const value = this.resume();
  const node = this.stack[this.stack.length - 1];
  if (node?.type === "math" || node?.type === "inlineMath") {
    node.value = node.type === "math" ? value.trim() : value;
  }
  this.exit(token);
};

const exitData: Handle = function (token) {
  this.config.enter.data!.call(this, token);
  this.config.exit.data!.call(this, token);
};

const fromMarkdown: FromMarkdownExtension = {
  enter: { latexMathInline: enterMath, latexMathDisplay: enterMath },
  exit: { latexMathInline: exitMath, latexMathDisplay: exitMath, latexMathData: exitData },
};

// micromark represents line endings and expanded tabs with negative codes.
function lineEnding(code: Code): boolean {
  return code === -5 || code === -4 || code === -3;
}

function space(code: Code): boolean {
  return code === 32 || code === -2 || code === -1;
}

/** Do not let an unclosed display block swallow text outside its list/quote. */
const continuation: Construct = {
  partial: true,
  tokenize(effects, ok, nok) {
    const context = this;
    return start;
    function start(code: Code): State | undefined {
      effects.enter("lineEnding");
      effects.consume(code);
      effects.exit("lineEnding");
      return next;
    }
    function next(code: Code): State | undefined {
      return context.parser.lazy[context.now().line] ? nok(code) : ok(code);
    }
  },
};

function tokenizeMath(display: boolean): Tokenizer {
  return function (effects, ok, nok) {
    const context = this;
    const type = display ? "latexMathDisplay" : "latexMathInline";
    const open = display ? 91 : 40;
    const close = display ? 93 : 41;
    const closing: Construct = { partial: true, tokenize: tokenizeClose };
    return start;

    function start(code: Code): State | undefined {
      effects.enter(type);
      effects.enter("latexMathMarker");
      effects.consume(code); // backslash
      return opening;
    }

    function opening(code: Code): State | undefined {
      if (code !== open) return nok(code);
      effects.consume(code);
      effects.exit("latexMathMarker");
      return afterOpening;
    }

    function afterOpening(code: Code): State | undefined {
      // Block math may interrupt a paragraph, just like a $$ fence.
      if (display && context.interrupt) return ok(code);
      return between(code);
    }

    function between(code: Code): State | undefined {
      // Like dollar fences, unfinished display math remains one live block.
      // Unmatched inline delimiters instead fall back to ordinary Markdown.
      if (code === null) return display ? done(code) : nok(code);
      if (lineEnding(code)) {
        if (display) return effects.attempt(continuation, between, done)(code);
        effects.enter("lineEnding");
        effects.consume(code);
        effects.exit("lineEnding");
        return between;
      }
      if (code === 92) return effects.attempt(closing, done, escape)(code);
      effects.enter("latexMathData");
      return content(code);
    }

    function content(code: Code): State | undefined {
      if (code === null || code === 92 || lineEnding(code)) {
        effects.exit("latexMathData");
        return between(code);
      }
      effects.consume(code);
      return content;
    }

    function escape(code: Code): State | undefined {
      effects.enter("latexMathData");
      effects.consume(code);
      return escaped;
    }

    function escaped(code: Code): State | undefined {
      // Consume pairs so a TeX line break (\\\\) cannot start a delimiter.
      if (code !== null && !lineEnding(code)) effects.consume(code);
      effects.exit("latexMathData");
      return code === null || lineEnding(code) ? between(code) : between;
    }

    function done(code: Code): State | undefined {
      effects.exit(type);
      return ok(code);
    }

    function tokenizeClose(closeEffects: Parameters<Tokenizer>[0], closeOk: State, closeNok: State): State {
      return begin;
      function begin(code: Code): State | undefined {
        closeEffects.enter("latexMathMarker");
        closeEffects.consume(code);
        return marker;
      }
      function marker(code: Code): State | undefined {
        if (code !== close) return closeNok(code);
        closeEffects.consume(code);
        return after;
      }
      function after(code: Code): State | undefined {
        if (display && space(code)) {
          closeEffects.consume(code);
          return after;
        }
        // A display block's closing delimiter must end its line.
        if (display && code !== null && !lineEnding(code)) return closeNok(code);
        closeEffects.exit("latexMathMarker");
        return closeOk(code);
      }
    }
  };
}
