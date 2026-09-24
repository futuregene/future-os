// Style B, "before" panel — what a real phone shows today: two floating
// controls for one transcript.
//
// The right-edge stack (shipped) plus the centred "回到最新" pill that lives in
// ComposerDock and appears whenever the reader is off the tail. On a real phone
// a drag sets that flag, so both are on screen at once.
//
// The pill here is a mock-up: `onScrollBeginDrag` has no equivalent in
// react-native-web, so a browser harness can never produce the real one (this is
// why the shipped panels in the other variants show no pill). It is rebuilt from
// FloatingTimelineButton's own tokens — 44px min height, 1px #e8edf4 border, pill
// radius, 12px horizontal padding, 13px/600 #5d687a label, the same shadow — and
// parked where that component parks itself (56px above the composer dock).
const down = document.querySelector('[aria-label="下一个提问"]');
const up = document.querySelector('[aria-label="上一个提问"]');
if (!down || !up) throw new Error('question nav control not found');
if (down.parentElement.children.length !== 2)
  throw new Error('this panel is the "before" state: the stack should still hold two buttons');

const input = document.querySelector('textarea');
if (!input) throw new Error('composer not found: the pill has to sit above it');

const pill = document.createElement('div');
pill.setAttribute('aria-label', '回到最新（示意）');
pill.innerHTML =
  '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="#5d687a" '
  + 'stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
  + '<path d="M12 5v14"/><path d="M6 13l6 6 6-6"/></svg><span>回到最新</span>';
Object.assign(pill.style, {
  position: 'fixed',
  left: '50%',
  transform: 'translateX(-50%)',
  display: 'flex',
  flexDirection: 'row',
  alignItems: 'center',
  gap: '4px',
  minHeight: '44px',
  padding: '8px 12px',
  boxSizing: 'border-box',
  border: '1px solid #e8edf4',
  borderRadius: '999px',
  background: '#ffffff',
  color: '#5d687a',
  font: '600 13px/1 system-ui, sans-serif',
  boxShadow: '0 3px 8px rgba(15, 23, 42, 0.08)',
  zIndex: 5,
});
// 56px above the dock's top edge, i.e. 12px above the composer's own top.
const composerTop = input.getBoundingClientRect().top - 4;
pill.style.top = `${Math.round(composerTop - 12 - 44)}px`;
document.body.appendChild(pill);
