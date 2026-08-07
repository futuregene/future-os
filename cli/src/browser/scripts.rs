//! Injected page scripts — port of `cli/src/browser/scripts/snapshot-script.ts`
//! and `cli/src/browser/scripts/console-hook-script.ts`.
//!
//! These are verbatim JS strings executed in page context. Keep them
//! byte-identical to the TS sources so both CLIs inject the same script.

/// `SNAPSHOT_FUNCTION_SOURCE` — function declaration for Runtime.evaluate.
pub const SNAPSHOT_FUNCTION_SOURCE: &str = r#"function(limit) {
  var escapeCss = function(value) {
    var css = globalThis.CSS;
    return css && css.escape ? css.escape(value) : value.replace(/["\\]/g, '\\$&');
  };
  var textOf = function(element) {
    if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
      return element.getAttribute('aria-label') ||
        element.getAttribute('placeholder') ||
        // Look for an associated <label> element
        (element.id ? (function() {
          var label = document.querySelector('label[for="' + element.id + '"]');
          return label ? (label.textContent || '').replace(/\s+/g, ' ').trim() : null;
        })() : null) ||
        element.name ||
        (element.value || '').slice(0, 30) ||
        '';
    }
    if (element instanceof HTMLImageElement) return element.alt || element.title || '';
    return element.getAttribute('aria-label') ||
      element.getAttribute('title') ||
      (element.textContent || '').replace(/\s+/g, ' ').trim();
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

  // ── Phase 1: interactive elements ──────────────────────────────────
  var interactiveSelector = 'a[href],button,input,textarea,select,summary,[role],[contenteditable="true"],[tabindex]';
  var candidates = Array.from(document.querySelectorAll(interactiveSelector));
  var items = [];
  var counter = 1;
  var seenSelectors = {};
  for (var i = 0; i < candidates.length; i++) {
    if (items.length >= limit) break;
    var element = candidates[i];
    if (!isVisible(element)) continue;
    var tag = element.tagName.toLowerCase();
    var role = roleOf(element);
    var name = textOf(element).slice(0, 120);
    if (!name && tag !== 'input' && tag !== 'textarea' && tag !== 'select') continue;
    var prefix = role === 'button' ? 'b' : role === 'textbox' ? 'i' : role === 'link' ? 'a' : 'e';
    var sel = uniqueSelector(element);
    items.push({
      ref: '' + prefix + (counter++),
      selector: sel,
      role: role,
      name: name,
      tag: tag,
      disabled: !!(element.disabled),
      checked: element instanceof HTMLInputElement && (element.type === 'checkbox' || element.type === 'radio')
        ? element.checked : null,
      href: element instanceof HTMLAnchorElement ? element.href : null,
    });
    seenSelectors[sel] = true;
  }

  // ── Phase 2: text containers ───────────────────────────────────────
  // Interactive-only snapshots miss headings, paragraphs, list items,
  // table cells — content the agent needs to understand the page.
  // Collect visible text-bearing non-interactive elements until the
  // limit is reached, skipping those already covered in Phase 1.
  var textTags = ['h1','h2','h3','h4','h5','h6','p','li','td','th','dt','dd','label','span','div','pre','code','blockquote','figcaption','legend'];
  for (var ti = 0; ti < textTags.length && items.length < limit; ti++) {
    var textCandidates = Array.from(document.querySelectorAll(textTags[ti]));
    for (var tj = 0; tj < textCandidates.length && items.length < limit; tj++) {
      var tel = textCandidates[tj];
      if (!isVisible(tel)) continue;
      // Skip elements that are (or contain) an interactive element from Phase 1
      if (tel.querySelector(interactiveSelector)) continue;
      var tsel = uniqueSelector(tel);
      if (seenSelectors[tsel]) continue;
      var ttext = (tel.textContent || '').replace(/\s+/g, ' ').trim();
      if (!ttext || ttext.length < 2) continue;
      seenSelectors[tsel] = true;
      items.push({
        ref: 't' + (counter++),
        selector: tsel,
        role: 'text',
        name: ttext.slice(0, 120),
        tag: tel.tagName.toLowerCase(),
        disabled: false,
        checked: null,
        href: null,
      });
    }
  }
  return {
    title: document.title,
    url: location.href,
    items: items,
  };
}"#;

/// `CONSOLE_HOOK_FUNCTION_SOURCE` — function declaration form.
pub const CONSOLE_HOOK_FUNCTION_SOURCE: &str = r#"function() {
  var target = globalThis;
  if (target.__futureConsoleHookInstalled) return;
  target.__futureConsoleHookInstalled = true;
  target.__futureConsoleLogs = target.__futureConsoleLogs || [];
  var levels = ['log', 'info', 'warn', 'error'];
  for (var li = 0; li < levels.length; li++) {
    var level = levels[li];
    var original = target.console[level].bind(target.console);
    target.console[level] = function() {
      var parts = [];
      for (var ai = 0; ai < arguments.length; ai++) {
        var arg = arguments[ai];
        try {
          parts.push(typeof arg === 'string' ? arg : JSON.stringify(arg));
        } catch (e) {
          parts.push(String(arg));
        }
      }
      target.__futureConsoleLogs.push({
        level: level,
        text: parts.join(' '),
        time: new Date().toISOString(),
      });
      if (target.__futureConsoleLogs.length > 200) target.__futureConsoleLogs.shift();
      original.apply(this, arguments);
    };
  }
}"#;

/// `CONSOLE_HOOK_INVOCATION_SOURCE` — IIFE form for
/// Page.addScriptToEvaluateOnNewDocument: `(function() {...})()`.
pub fn console_hook_invocation_source() -> String {
    format!("({})()", CONSOLE_HOOK_FUNCTION_SOURCE)
}

/// Snapshot item shape returned by the page script.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SnapshotItem {
    pub r#ref: String,
    pub selector: String,
    pub role: String,
    pub name: String,
    pub tag: String,
    pub disabled: bool,
    pub checked: Option<bool>,
    pub href: Option<String>,
}

/// `SnapshotResult`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SnapshotResult {
    pub title: String,
    pub url: String,
    pub items: Vec<SnapshotItem>,
}
