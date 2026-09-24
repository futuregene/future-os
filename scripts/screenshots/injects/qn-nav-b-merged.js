// Style B, merged — "back to latest" folds into the arrow stack as a third
// round button, so the transcript has exactly one floating control.
//
// Mock-up overlay for the demo sheet: the button is fabricated in the browser,
// not shipped. It is built by cloning the ↑ button, so every pixel of it (44×44,
// 1px #e8edf4 border, 22px radius, the same shadow) is the product's own
// button — only the glyph and the label are new.
//
// The double chevron, not a third single arrow: a plain ↓ next to the existing
// ↓ reads as a duplicated control.
const up = document.querySelector('[aria-label="上一个提问"]');
const down = document.querySelector('[aria-label="下一个提问"]');
const stack = up && up.parentElement;

if (!up || !down || !stack) throw new Error('question nav control not found');
if (getComputedStyle(stack).flexDirection !== 'column')
  throw new Error(`expected a vertical stack, got ${getComputedStyle(stack).flexDirection}`);
if (up.parentElement !== down.parentElement)
  throw new Error('the arrows are not in one container');

// Clone the *enabled* button: in some panels ↓ is inert, and "back to latest"
// carries its own state (it is live whenever the reader is off the tail, which
// is exactly when this stack is on screen).
const latest = up.cloneNode(true);
latest.setAttribute('aria-label', '回到最新');
latest.removeAttribute('aria-disabled');
latest.removeAttribute('disabled');
latest.innerHTML =
  '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="#172033" '
  + 'stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">'
  + '<path d="M7 6l5 5 5-5"/><path d="M7 12l5 5 5-5"/></svg>';
stack.appendChild(latest);

// The properties this panel is meant to prove: one column, three buttons, all
// 44px, all circular, 8px apart.
const buttons = [...stack.children];
if (buttons.length !== 3)
  throw new Error(`expected three buttons after merging, got ${buttons.length}`);
for (const button of buttons) {
  const rect = button.getBoundingClientRect();
  if (Math.round(rect.width) !== 44 || Math.round(rect.height) !== 44)
    throw new Error(`a merged button is ${Math.round(rect.width)}×${Math.round(rect.height)}, expected 44×44`);
  if (parseFloat(getComputedStyle(button).borderTopLeftRadius) < 20)
    throw new Error('a merged button is not circular');
}
const gap = buttons[1].getBoundingClientRect().top - buttons[0].getBoundingClientRect().bottom;
if (Math.round(gap) !== 8) throw new Error(`buttons sit ${Math.round(gap)}px apart, expected 8`);
