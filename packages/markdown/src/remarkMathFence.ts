import type { Root } from "mdast";
import type { Code, Construct, ConstructRecord, Effects, State, TokenizeContext } from "micromark-util-types";
import type { Plugin } from "unified";

declare module "micromark-util-types" {
  interface TokenTypeMap {
    mathFlow: "mathFlow";
    mathFlowFence: "mathFlowFence";
    mathFlowFenceSequence: "mathFlowFenceSequence";
    mathFlowValue: "mathFlowValue";
  }
}

/**
 * `$$…$$` display math as models actually write it.
 *
 * remark-math's block rule reads a same-line tail after `$$` as fence *meta*
 * (the asciimath form), and its closing fence has to be a line that is nothing
 * but `$$`. A reply that opens on the first formula line and closes on the last
 * one — the shape most models emit —
 *
 *     $$N^*(L) = \frac{…}{…}
 *     = \frac{…}{…}\;+\;\text{小项}$$
 *
 * therefore loses its opening line into `meta` and never finds a closing fence:
 * the `math` node swallows every block after it, so a table further down came
 * back as literal `|` rows and the rest of the reply was painted as the
 * formula's source (the `$$` was still visible in the fallback). A single-line
 * `$$x$$` looked fine only by accident — its block fence failed too and the
 * paragraph fell back to *inline* math, which `parseFutureMarkdown` then
 * promotes back to a block.
 *
 * This construct replaces the block rule with one that stays offset-exact (no
 * source rewriting, so streaming still slices the same text):
 *
 * 1. Content may start on the opening line — it is never `meta`.
 * 2. A closing fence is a `$` run at least as long as the opening one, at any
 *    indentation, followed only by spaces and the end of its line; so line-final
 *    `…\text{小项}$$` closes.
 * 3. A fence-length `$` run that does *not* end its line (``$$x$$ and more``) is
 *    not a display block: the construct gives up and the paragraph path renders
 *    it as inline math, exactly as it did before.
 *
 * `unified` tries syntax extensions registered later first, so this wins over
 * remark-math's rule for the opening `$$`; remark-math's *inline* rule for `$`
 * is left alone.
 */
export const remarkMathFence: Plugin<[], Root> = function () {
  const data = this.data();
  (data.micromarkExtensions ??= []).push({
    flow: { 36: mathFence } as ConstructRecord,
  });
};

// micromark represents line endings and expanded tabs with negative codes.
function lineEnding(code: Code): boolean {
  return code === -5 || code === -4 || code === -3;
}

function space(code: Code): boolean {
  return code === 32 || code === -2 || code === -1;
}

/** Consume up to `max` spaces, wrapped in a `whitespace` token the way
 * micromark's own `factorySpace` does — `effects.consume` must run with a token
 * open, and the wrapper keeps the spaces out of the formula (no mdast handler
 * reads it). */
function factorySpace(effects: Effects, ok: State, max = Number.POSITIVE_INFINITY): State {
  let size = 0;
  return start;
  function start(code: Code): State | undefined {
    // Only open the token when it will hold at least one space: micromark
    // rejects an empty one.
    if (size < max && space(code)) {
      effects.enter("whitespace");
      return prefix(code);
    }
    return ok(code);
  }
  function prefix(code: Code): State | undefined {
    if (space(code) && size++ < max) {
      effects.consume(code);
      return prefix;
    }
    effects.exit("whitespace");
    return ok(code);
  }
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

function tokenizeMathFence(this: TokenizeContext, effects: Effects, ok: State, nok: State): State {
  const context = this;
  const tail = context.events[context.events.length - 1];
  const initialSize = tail && tail[1].type === "linePrefix" ? tail[2].sliceSerialize(tail[1], true).length : 0;
  let sizeOpen = 0;
  let runSize = 0;

  function start(code: Code): State | undefined {
    effects.enter("mathFlow");
    effects.enter("mathFlowFence");
    effects.enter("mathFlowFenceSequence");
    return openSequence(code);
  }

  function openSequence(code: Code): State | undefined {
    if (code === 36) {
      effects.consume(code);
      sizeOpen++;
      return openSequence;
    }
    // `$` alone is inline math, not a fence.
    if (sizeOpen < 2) return nok(code);
    effects.exit("mathFlowFenceSequence");
    // Closing the opening fence starts mdast's content buffer; whitespace
    // between the fence and the formula is not part of it.
    effects.exit("mathFlowFence");
    return factorySpace(effects, beforeContent)(code);
  }

  function contentStart(code: Code): State | undefined {
    // A continuation line may carry the same indentation as the opening fence
    // (this is what remark-math's `factorySpace` prefix does); the indentation
    // is not formula text. Deeper indentation is content.
    if (code === 36) return effects.attempt(closingFence, after, notClosing)(code);
    if (code === null || lineEnding(code)) return beforeContent(code);
    effects.enter("mathFlowValue");
    return content(code);
  }

  /** At the start of a line or of a formula run: only a closing fence or the
   * content itself can follow. */
  function beforeContent(code: Code): State | undefined {
    if (code === null) return after(code);
    if (lineEnding(code)) return effects.attempt(continuation, beforeContent, after)(code);
    if (space(code)) return factorySpace(effects, contentStart, initialSize)(code);
    return contentStart(code);
  }

  function content(code: Code): State | undefined {
    if (code === null || lineEnding(code)) {
      effects.exit("mathFlowValue");
      return beforeContent(code);
    }
    if (code === 36) {
      effects.exit("mathFlowValue");
      return effects.attempt(closingFence, after, notClosing)(code);
    }
    effects.consume(code);
    return content;
  }

  /** A `$` run that did not close the block. A run shorter than the fence is
   * ordinary formula text; a fence-length run followed by more text means the
   * author wrote inline math (``$$x$$ and more``), so the whole block gives up
   * and the paragraph path handles it. */
  function notClosing(code: Code): State | undefined {
    effects.enter("mathFlowValue");
    runSize = 0;
    return dollarRun(code);
  }

  function dollarRun(code: Code): State | undefined {
    if (code === 36) {
      runSize++;
      effects.consume(code);
      return dollarRun;
    }
    if (runSize >= sizeOpen) return nok(code);
    runSize = 0;
    return content(code);
  }

  /** Attempted at a `$` run; succeeds only when the run is at least as long as
   * the opening fence and the line ends right after it. */
  const closingFence: Construct = {
    partial: true,
    tokenize(closeEffects, closeOk, closeNok) {
      let size = 0;
      return beforeSequence;
      function beforeSequence(code: Code): State | undefined {
        closeEffects.enter("mathFlowFence");
        closeEffects.enter("mathFlowFenceSequence");
        return sequence(code);
      }
      function sequence(code: Code): State | undefined {
        if (code === 36) {
          size++;
          closeEffects.consume(code);
          return sequence;
        }
        if (size < sizeOpen) return closeNok(code);
        closeEffects.exit("mathFlowFenceSequence");
        // Trailing whitespace is the container's, not the formula's.
        return factorySpace(closeEffects, afterSequence)(code);
      }
      function afterSequence(code: Code): State | undefined {
        if (code === null || lineEnding(code)) {
          closeEffects.exit("mathFlowFence");
          return closeOk(code);
        }
        return closeNok(code);
      }
    },
  };

  function after(code: Code): State | undefined {
    effects.exit("mathFlow");
    return ok(code);
  }

  return start;
}

const mathFence: Construct = {
  tokenize: tokenizeMathFence,
  // Like remark-math's block rule: containers may not pierce the formula.
  concrete: true,
  name: "mathFence",
};
