// Style C (proposal) — the stack carries the question number.
//
// Answers "which question am I in, out of how many" while jumping. The number is
// a proposal only: this demo is at the newest of four turns. If this shape is
// chosen, the component has to derive the ordinal from the anchor question — the
// implementation deliberately tracks positions, not numbers.
const down = document.querySelector('[aria-label="下一个提问"]');
const stack = down && down.parentElement;

if (!down || !stack) throw new Error('question nav control not found');

const counter = document.createElement('div');
counter.textContent = '4 / 4';
Object.assign(counter.style, {
  width: '44px',
  height: '26px',
  boxSizing: 'border-box',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  marginBottom: '8px',
  border: '1px solid #e8edf4',
  borderRadius: '999px',
  background: '#ffffff',
  color: '#707c90',
  font: '600 12px/1 system-ui, sans-serif',
  fontVariantNumeric: 'tabular-nums',
  boxShadow: '0 3px 10px rgba(15, 23, 42, 0.10)',
});
stack.insertBefore(counter, stack.firstChild);
