// Style B, merged — the state the arrows alone cannot show: the reader has
// landed on the last question, so ↓ has nothing below it and goes inert, while
// "back to latest" is still live because the tail of the answer is still below.
//
// This is the panel that shows why merging fits: the pill is on screen exactly
// when this stack is on screen, so the third button inherits a state rule that
// already exists rather than adding one.
//
// The walk to the last question drives the real control (`↓.click()`) instead of
// painting on an inert look, so the shot shows the product's own state. Injects
// cannot await, so the walk runs as a chain and publishes `__mergedLastQuestion`
// for the scenario's assertion step to read.
const down = document.querySelector('[aria-label="下一个提问"]');
const up = document.querySelector('[aria-label="上一个提问"]');
if (!down || !up) throw new Error('question nav control not found');

// Appended only once the walk is over: the clicks re-render the stack, and a
// node inserted mid-render is the one React would drop.
const appendLatest = () => {
  const target = document.querySelector('[aria-label="下一个提问"]').parentElement;
  const latest = document.querySelector('[aria-label="上一个提问"]').cloneNode(true);
  latest.setAttribute('aria-label', '回到最新');
  latest.innerHTML =
    '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="#172033" '
    + 'stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">'
    + '<path d="M7 6l5 5 5-5"/><path d="M7 12l5 5 5-5"/></svg>';
  target.appendChild(latest);
};

window.__mergedLastQuestion = false;
const read = label => document.querySelector(`[aria-label="${label}"]`);
const walk = async () => {
  for (let i = 0; i < 8; i += 1) {
    if (read('下一个提问').getAttribute('aria-disabled') === 'true') break;
    read('下一个提问').click();
    await new Promise(resolve => setTimeout(resolve, 700));
  }
  if (read('下一个提问').getAttribute('aria-disabled') !== 'true')
    throw new Error('↓ never went inert: the demo transcript should end after four questions');
  if (read('上一个提问').getAttribute('aria-disabled') === 'true')
    throw new Error('↑ should stay available on the last question');
  // The double chevron, not a third single arrow: a plain ↓ next to the existing
  // ↓ reads as a duplicated control.
  appendLatest();
  window.__mergedLastQuestion = true;
};
void walk();
