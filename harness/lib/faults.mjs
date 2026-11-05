/**
 * Test-only fault symptoms.
 *
 * Two halves, both selected by `harness/cli.mjs run --fault <name>`:
 *
 *  - the fixture side (`fixtureFault`) makes the backend actually misbehave, so a
 *    case's assertions fail for a real reason;
 *  - the overlay side (`FAULT_OVERLAYS`) is registered as a document-start script,
 *    before the page navigates, so the *symptom* is on screen for the whole
 *    recording instead of appearing for one frame.
 *
 * Round one taught this the hard way: faults whose symptom is invisible
 * (`hang` looks like normal startup, `api-500` is swallowed into an empty
 * sidebar) measure nothing. A defect has to be pointable in the video.
 *
 * Nothing here runs unless a fault is named.
 */

export const FAULT_NAMES = ["api-500", "empty-list", "dead-click"];

const banner = (text) => `(function () {
  var KID = '__fault_banner__';
  function paint() {
    if (document.getElementById(KID)) return;
    if (!document.body) return;
    var b = document.createElement('div');
    b.id = KID;
    b.textContent = ${JSON.stringify(text)};
    b.style.cssText = 'position:fixed;left:50%;top:58px;transform:translateX(-50%);' +
      'background:#b3261e;color:#fff;font:14px -apple-system,sans-serif;padding:10px 18px;' +
      'border-radius:8px;box-shadow:0 2px 12px rgba(0,0,0,.28);z-index:2147483647';
    document.body.appendChild(b);
  }
  paint(); setInterval(paint, 400);
})();`;

export const FAULT_OVERLAYS = {
  // a red banner that outlives every re-render
  "api-500": banner("同步失败：无法加载会话列表（500）"),

  // the list area is blank AND says so: two symptoms that agree, which is what
  // makes an empty state pointable instead of merely quiet
  "empty-list": `(function () {
    function paint() {
      if (!document.body) return;
      var rail = document.querySelector('[data-testid="rail-view-band"]');
      if (rail && rail.parentElement && !document.getElementById('__fault_mask__')) {
        var r = rail.parentElement.getBoundingClientRect();
        var mask = document.createElement('div');
        mask.id = '__fault_mask__';
        mask.style.cssText = 'position:fixed;left:' + r.left + 'px;top:' + r.top + 'px;' +
          'width:' + r.width + 'px;height:' + Math.max(160, innerHeight - r.top - 120) + 'px;' +
          'background:#fff;z-index:2147483640';
        document.body.appendChild(mask);
      }
      if (!document.getElementById('__fault_empty__')) {
        var d = document.createElement('div');
        d.id = '__fault_empty__';
        d.textContent = '暂无工作区，暂无会话';
        d.style.cssText = 'position:fixed;left:24px;top:320px;color:#9aa0a6;' +
          'font:13px -apple-system;z-index:2147483641';
        document.body.appendChild(d);
      }
    }
    paint(); setInterval(paint, 400);
  })();`,

  // the primary action stays greyed out with a spinner that never ends
  "dead-click": `(function () {
    function paint() {
      if (!document.body) return;
      var buttons = [].slice.call(document.querySelectorAll('button'));
      buttons.forEach(function (b) {
        var label = (b.innerText || '') + (b.getAttribute('aria-label') || '');
        if (/发送|Send/.test(label)) { b.style.pointerEvents = 'none'; b.style.opacity = '0.55'; }
      });
      if (document.getElementById('__fault_spin__')) return;
      var d = document.createElement('div');
      d.id = '__fault_spin__';
      d.textContent = '⟳ 正在发送，未响应…';
      d.style.cssText = 'position:fixed;right:24px;bottom:110px;background:#fef7e0;color:#b06000;' +
        'font:13px -apple-system;padding:6px 12px;border-radius:14px;z-index:2147483646';
      document.body.appendChild(d);
    }
    paint(); setInterval(paint, 400);
  })();`,
};

/** The fixture-side half: which commands misbehave, and how. */
export const FIXTURE_FAULTS = {
  "api-500": {
    on: ["list_threads", "list_threads_page", "list_pinned_threads"],
    payload: () => ({}),
    error: "fault(injected): upstream returned 500",
  },
  "empty-list": {
    on: ["list_threads", "list_threads_page", "list_pinned_threads", "list_workspaces"],
    payload: (command) => ({
      value: command === "list_threads_page"
        ? { threads: [], nextCursor: null, totalRoots: 0 }
        : [],
    }),
  },
  // nothing changes on the backend, but the control never responds
  "dead-click": {
    on: ["send_message", "submit_message", "agent_send"],
    payload: () => ({}),
    hangMs: 30000,
  },
};

export function applyFixtureFault(fault, command) {
  if (!fault) return null;
  const spec = FIXTURE_FAULTS[fault];
  if (!spec || !spec.on.includes(command)) return null;
  return { payload: spec.payload(command), error: spec.error ?? null, hangMs: spec.hangMs ?? 0 };
}
