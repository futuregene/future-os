/**
 * Console hook script.
 *
 * Injects console.log/info/warn/error interception into the page.
 * Buffers up to 200 messages in window.__futureConsoleLogs.
 *
 * Two forms:
 * - CONSOLE_HOOK_FUNCTION_SOURCE: function declaration for Runtime.callFunctionOn
 * - CONSOLE_HOOK_INVOCATION_SOURCE: IIFE for Page.addScriptToEvaluateOnNewDocument
 */
export const CONSOLE_HOOK_FUNCTION_SOURCE = `function() {
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
}`;

/** IIFE form — for use with Page.addScriptToEvaluateOnNewDocument. */
export const CONSOLE_HOOK_INVOCATION_SOURCE = `(${CONSOLE_HOOK_FUNCTION_SOURCE})()`;
