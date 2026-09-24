// Style B (proposal) — the stack grows a third round button for "back to
// latest".
//
// On a real phone, dragging the transcript away from the tail also reveals the
// centred "回到最新" pill above the composer, so the reader sees two floating
// controls. This panel shows folding that action into the stack instead, leaving
// one control for the whole transcript.
//
// Mock-up overlay for the demo sheet (see qn-nav-a-stacked.js): the button is
// fabricated in the browser, not shipped.
const down = document.querySelector('[aria-label="下一个提问"]');
const stack = down && down.parentElement;

if (!down || !stack) throw new Error('question nav control not found');

const latest = down.cloneNode(false);
latest.removeAttribute('aria-label');
for (const child of [...latest.childNodes]) child.remove();
// Two chevrons ("jump to the end"), not a third single arrow: a plain ↓ next to
// the existing ↓ reads as a duplicated control.
latest.innerHTML =
  '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="#2563eb" '
  + 'stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">'
  + '<path d="M7 6l5 5 5-5"/><path d="M7 12l5 5 5-5"/></svg>';
stack.appendChild(latest);
