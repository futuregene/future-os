import type { Root } from "mdast";
import type { Code, Construct, ConstructRecord, State } from "micromark-util-types";
import type { Plugin } from "unified";

/**
 * Money is not maths: `$0.20/$0.60` must stay text.
 *
 * `remark-math` reads every `$…$` pair as inline math, so a reply that quotes
 * prices — `DeepInfra Standard $0.20/$0.60、缓存读 $0.0048` — turns whatever
 * sits *between* two dollar signs into a formula. The span is usually prose:
 * the KaTeX renderer then typesets the Chinese as symbols and the paragraph
 * visibly collapses into a run of raised, shrunken glyphs (measured on a real
 * pricing report: 13 such spans, e.g. `0.0048–0.006；OpenRouter 上 19 家`).
 *
 * A currency amount is recognisable without looking at its partner: the `$` is
 * glued to a number (`$10`, `$0.20`, `$1,200`). So this construct claims
 * exactly those `$` — plus the two characters after it — as `data`, which the
 * mdast conversion turns into ordinary text. Everything else is left to
 * `remark-math`: an opening `$$` (display) and any `$` followed by something
 * that is not a number fall through with `nok`, so `$a+b$`, `$2^n$` and
 * `$2\pi$` still render as formulas.
 *
 * Claiming only the sigil — rather than pairing it with its partner — keeps
 * emphasis inside a quoted amount intact: in `$10 买到的是**一份 $100 额度**`
 * the `**…**` is parsed by the normal emphasis pass instead of being frozen
 * into a literal text run.
 *
 * The trade-off is the one every engine that guards currency accepts: an
 * inline formula whose body starts with two numeric characters, such as
 * `$1.5x$`, is read as text. Display math (`$$…$$`, `\(…\)`) is unaffected.
 */
export const remarkCurrencyMath: Plugin<[], Root> = function () {
  const data = this.data();
  (data.micromarkExtensions ??= []).push({ text: { 36: currencyDollar } as ConstructRecord });
};

function digit(code: Code): boolean {
  return code !== null && code >= 48 && code <= 57;
}

/** CJK ideographs (incl. extension A) and CJK punctuation: text, not TeX. */
function cjk(code: Code): boolean {
  return code !== null && ((code >= 0x2e80 && code <= 0x9fff) || (code >= 0xf900 && code <= 0xfaff));
}

/**
 * `$` + digit + (digit | `.` | `,`) — an amount: `$10`, `$0.20`, `$1,200`.
 * `$` + digit + space + CJK — a single-digit amount inside Chinese prose,
 * `$5 首月`; Latin after the space stays math (`$5 x$`, `$2 \\pi$`).
 */
const currencyDollar: Construct = {
  name: "currencyDollar",
  tokenize(effects, ok, nok) {
    return start;

    /** `code` is the `$` that opened this attempt. */
    function start(code: Code): State | undefined {
      effects.enter("data");
      effects.consume(code);
      return afterDollar;
    }

    function afterDollar(code: Code): State | undefined {
      if (!digit(code)) return nok(code);
      effects.consume(code);
      return afterDigit;
    }

    function afterDigit(code: Code): State | undefined {
      if (digit(code) || code === 46 || code === 44) {
        effects.consume(code);
        return done;
      }
      if (code === 32) {
        effects.consume(code);
        return afterSpace;
      }
      return nok(code);
    }

    function afterSpace(code: Code): State | undefined {
      if (!cjk(code)) return nok(code);
      effects.consume(code);
      return done;
    }

    function done(code: Code): State | undefined {
      effects.exit("data");
      // `code` is the first character after the amount and stays unconsumed.
      return ok(code);
    }
  },
};
