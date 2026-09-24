// State 1 for the demo sheet: scrolled up, nothing pressed yet.
//
// The reader is inside a long answer, so the transcript is longer than the
// viewport and the control is on screen. The checks are the point: "the control
// appears when the reader scrolls away" has to be true for this panel to mean
// anything.
const up = document.querySelector('[aria-label="上一个提问"]');
const down = document.querySelector('[aria-label="下一个提问"]');

if (!up || !down) throw new Error('question nav control is not on screen after scrolling');
if (up.getAttribute('aria-disabled') === 'true')
  throw new Error('↑ should be offered while reading inside a turn');
