/**
 * Upright probe: put the transcript's own text next to an unflipped copy of
 * itself, so one screenshot can be judged without a human reading it.
 *
 * `capture.py check-upright-mobile <scenario>` runs a scenario with this file
 * injected before the final shot and then measures the two marked tiles:
 *
 *   - the tile in place (the transcript, 3 px magenta outline) — whatever the
 *     app really renders, which under react-native-web's inverted list is
 *     upside down unless the harness neutralises the extra flip;
 *   - the same text cloned into `document.body` (3 px cyan outline) — outside
 *     every flipped ancestor, so it is upright by construction.
 *
 * Both tiles are the *same text at the same width and typography*, so they
 * differ only in orientation: correlating them straight and vertically mirrored
 * says which way up the capture is, and by how much. A clone is also the only
 * control that needs no second browser, no device and no OCR — see
 * `check-upright` in docs/guide/screenshots.md.
 */
(() => {
  const outline = (el, colour) => { el.style.outline = `3px solid ${colour}`; };

  // The scroller that is *visible*, not merely scrollable.
  //
  // The phone keeps the session list mounted underneath an open conversation
  // (`App.tsx` hides it with `pointerEvents: none` rather than unmounting it), so
  // a rail long enough to scroll is a second scroller of its own — and a taller
  // one than the transcript. Picking by height alone then outlined a row of the
  // list behind the chat: the marker never reached the picture and the check
  // reported "no in-place tile" for a transcript that was perfectly upright.
  // A hit test at the scroller's own centre tells the two apart without knowing
  // either of them: the list's centre is answered by the chat drawn over it.
  const frontmost = (el) => {
    const r = el.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2, Math.min(r.bottom - 8, r.top + r.height / 2));
    return !!hit && (el === hit || el.contains(hit));
  };
  const scroller = [...document.querySelectorAll('div')]
    .filter((el) => el.scrollHeight > el.clientHeight + 10 && el.clientHeight > 400 && frontmost(el))
    .sort((a, b) => b.getBoundingClientRect().height - a.getBoundingClientRect().height)[0];
  if (!scroller) throw new Error('upright probe: no scroller on screen');
  const viewport = scroller.getBoundingClientRect();

  // The largest block of text that is fully inside the list viewport: partially
  // visible text would correlate against a clipped copy of itself.
  //
  // "Block of text" includes a paragraph that carries inline runs — a sentence
  // with `code` in it, **bold**, a link — because that is what a reply is made
  // of: requiring a childless element picked a single 13 px code line, and one
  // line of monospace ink says nothing about which way up the transcript reads
  // (measured: 0.045 upright against 0.053 mirrored — inconclusive). A block
  // whose children are all inline-level is still one flowing paragraph, so it
  // clones and wraps as a whole; a flex/grid container is not, and is skipped.
  const inlineOnly = (el) => [...el.children].every((child) => {
    const display = getComputedStyle(child).display;
    return display === 'inline' || display === 'contents';
  });
  const inside = (r) => r.top > viewport.top + 4 && r.bottom < viewport.bottom - 4
    && r.left >= viewport.left && r.right <= viewport.right;
  const target = [...scroller.querySelectorAll('div,span,p')]
    .filter((el) => (el.innerText || '').trim().length > 20
      && (el.children.length === 0 || inlineOnly(el))
      && inside(el.getBoundingClientRect()))
    .sort((a, b) => {
      const ra = a.getBoundingClientRect();
      const rb = b.getBoundingClientRect();
      return (rb.width * rb.height) - (ra.width * ra.height);
    })[0];
  if (!target) throw new Error('upright probe: no fully visible text block in the transcript');
  const rect = target.getBoundingClientRect();

  // A clone in `document.body` inherits no ancestor transform, so it renders the
  // way the text is meant to read. Copy the typography the clone would otherwise
  // inherit from the bubble it came from, and give it the same content width, so
  // both tiles wrap identically.
  const computed = getComputedStyle(target);
  const style = ['fontFamily', 'fontSize', 'fontWeight', 'fontStyle', 'letterSpacing', 'lineHeight',
    'textAlign', 'direction', 'whiteSpace', 'wordSpacing', 'paddingLeft', 'paddingRight',
    'paddingTop', 'paddingBottom'];
  const control = document.createElement('div');
  control.setAttribute('data-upright', 'control');
  control.style.cssText = [
    'position:fixed', 'left:6px', 'bottom:6px', 'z-index:2147483647',
    `width:${Math.ceil(rect.width)}px`, 'background:#fff', 'color:#111',
    ...style.map((name) => `${name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}:${computed[name]}`),
  ].join(';');
  const clone = target.cloneNode(true);
  clone.style.width = '100%';
  clone.style.margin = '0';
  control.appendChild(clone);
  outline(control, 'rgb(0,255,255)');
  document.body.appendChild(control);
  outline(target, 'rgb(255,0,255)');

  const box = (el) => {
    const r = el.getBoundingClientRect();
    return [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)];
  };
  const report = {
    text: (target.innerText || '').trim().slice(0, 24),
    inPlace: box(target),
    control: box(control),
    flipsAboveInPlace: (() => {
      let n = 0;
      for (let el = target; el && el !== document.body; el = el.parentElement) {
        const m = getComputedStyle(el).transform.match(/^matrix\(([^)]+)\)$/);
        if (!m) continue;
        const [a, b, c, d, e, f] = m[1].split(',').map(Number);
        if (a === 1 && b === 0 && c === 0 && d === -1 && e === 0 && f === 0) n += 1;
      }
      return n;
    })(),
  };
  window.__upright = report;
  return report;
})()
