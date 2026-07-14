/**
 * Snapshot evaluation script.
 *
 * Executed in page context to produce the accessibility-tree-like
 * snapshot of interactive elements. Must be pure ES5-compatible JS.
 *
 * Used as a function declaration for Runtime.callFunctionOn / page.evaluate.
 */
export const SNAPSHOT_FUNCTION_SOURCE = `function(limit) {
  var escapeCss = function(value) {
    var css = globalThis.CSS;
    return css && css.escape ? css.escape(value) : value.replace(/["\\\\]/g, '\\\\$&');
  };
  var textOf = function(element) {
    if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
      return element.getAttribute('aria-label') ||
        element.getAttribute('placeholder') ||
        element.name ||
        element.value || '';
    }
    if (element instanceof HTMLImageElement) return element.alt || element.title || '';
    return element.getAttribute('aria-label') ||
      element.getAttribute('title') ||
      (element.textContent || '').replace(/\\s+/g, ' ').trim();
  };
  var roleOf = function(element) {
    var explicit = element.getAttribute('role');
    if (explicit) return explicit;
    var tag = element.tagName.toLowerCase();
    if (tag === 'a') return 'link';
    if (tag === 'button') return 'button';
    if (tag === 'select') return 'combobox';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'summary') return 'button';
    if (tag === 'input') {
      var type = (element.getAttribute('type') || 'text').toLowerCase();
      if (type === 'button' || type === 'submit' || type === 'reset') return 'button';
      if (type === 'checkbox') return 'checkbox';
      if (type === 'radio') return 'radio';
      return 'textbox';
    }
    return tag;
  };
  var uniqueSelector = function(element) {
    var id = element.getAttribute('id');
    if (id && document.querySelectorAll('#' + escapeCss(id)).length === 1) return '#' + escapeCss(id);
    var attrs = ['data-testid', 'data-test', 'data-cy', 'name', 'aria-label'];
    for (var ai = 0; ai < attrs.length; ai++) {
      var attr = attrs[ai];
      var value = element.getAttribute(attr);
      if (!value) continue;
      var sel = element.tagName.toLowerCase() + '[' + attr + '="' + escapeCss(value) + '"]';
      if (document.querySelectorAll(sel).length === 1) return sel;
    }
    var parts = [];
    var current = element;
    while (current && current !== document.documentElement) {
      var tag = current.tagName.toLowerCase();
      var currentTag = current.tagName;
      var parent = current.parentElement;
      if (!parent) break;
      var siblings = Array.from(parent.children).filter(function(child) { return child.tagName === currentTag; });
      var index = siblings.indexOf(current) + 1;
      parts.unshift(siblings.length > 1 ? tag + ':nth-of-type(' + index + ')' : tag);
      var selector = parts.join(' > ');
      if (document.querySelectorAll(selector).length === 1) return selector;
      current = parent;
    }
    return parts.join(' > ');
  };
  var isVisible = function(element) {
    var rect = element.getBoundingClientRect();
    var style = getComputedStyle(element);
    return rect.width > 0 &&
      rect.height > 0 &&
      style.visibility !== 'hidden' &&
      style.display !== 'none' &&
      Number(style.opacity || '1') > 0;
  };

  var candidates = Array.from(document.querySelectorAll(
    'a[href],button,input,textarea,select,summary,[role],[contenteditable="true"],[tabindex]'
  ));
  var items = [];
  var counter = 1;
  for (var i = 0; i < candidates.length; i++) {
    if (items.length >= limit) break;
    var element = candidates[i];
    if (!isVisible(element)) continue;
    var tag = element.tagName.toLowerCase();
    var role = roleOf(element);
    var name = textOf(element).slice(0, 120);
    if (!name && tag !== 'input' && tag !== 'textarea' && tag !== 'select') continue;
    var prefix = role === 'button' ? 'b' : role === 'textbox' ? 'i' : role === 'link' ? 'a' : 'e';
    items.push({
      ref: '' + prefix + (counter++),
      selector: uniqueSelector(element),
      role: role,
      name: name,
      tag: tag,
      disabled: !!(element.disabled),
      checked: element instanceof HTMLInputElement && (element.type === 'checkbox' || element.type === 'radio')
        ? element.checked : null,
      href: element instanceof HTMLAnchorElement ? element.href : null,
    });
  }
  return {
    title: document.title,
    url: location.href,
    items: items,
  };
}`;

export interface SnapshotItem {
  ref: string;
  selector: string;
  role: string;
  name: string;
  tag: string;
  disabled: boolean;
  checked: boolean | null;
  href: string | null;
}

export interface SnapshotResult {
  title: string;
  url: string;
  items: SnapshotItem[];
}
