import "./mathjaxEnvironment";
import { mathjax } from "@mathjax/src/js/mathjax.js";
import { TeX } from "@mathjax/src/js/input/tex.js";
import { SVG } from "@mathjax/src/js/output/svg.js";
import { liteAdaptor } from "@mathjax/src/js/adaptors/liteAdaptor.js";
import { RegisterHTMLHandler } from "@mathjax/src/js/handlers/html.js";
import { MathJaxTexFont } from "@mathjax/mathjax-tex-font/js/svg.js";
import "@mathjax/src/js/input/tex/ams/AmsConfiguration.js";

const adaptor = liteAdaptor();
RegisterHTMLHandler(adaptor);
const tex = new TeX({ packages: ["base", "ams"], maxBuffer: 8_192, maxMacros: 1_000 });
const document = mathjax.document("", {
  InputJax: tex,
  // Embed glyph paths, not external fonts, DOM, CDN scripts or global SVG IDs.
  OutputJax: new SVG({ fontCache: "none", fontData: MathJaxTexFont }),
});

export interface MathSvg {
  xml: string;
  /** Dimensions in em; native font scaling is applied by the view. */
  width: number;
  height: number;
}
const cache = new Map<string, MathSvg | null>();

/** Offline TeX layout for native SVG. Incomplete/unsupported input stays source. */
export function renderMathSvg(code: string, display: boolean): MathSvg | null {
  if (!code.trim() || code.length > 8_192) return null;
  const key = `${display ? "block" : "inline"}:${code}`;
  if (cache.has(key)) return cache.get(key)!;
  let result: MathSvg | null = null;
  try {
    tex.reset();
    const container = document.convert(code, { display });
    const svg = adaptor.firstChild(container);
    if (svg && "attributes" in svg && adaptor.kind(svg) === "svg") {
      const xml = adaptor.outerHTML(svg);
      const bounds = adaptor.getAttribute(svg, "viewBox")?.split(/\s+/).map(Number);
      if (xml.length <= 256 * 1024 && bounds?.length === 4 && bounds.every(Number.isFinite)
        && bounds[2]! > 0 && bounds[3]! > 0 && !xml.includes('data-mml-node="merror"')) {
        result = { xml, width: bounds[2]! / 1_000, height: bounds[3]! / 1_000 };
      }
    }
  } catch { /* Keep malformed or partially streamed formulas readable as TeX. */ }
  if (cache.size >= 128) cache.delete(cache.keys().next().value!);
  cache.set(key, result);
  return result;
}
