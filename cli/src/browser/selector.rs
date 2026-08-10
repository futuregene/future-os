//! Selector resolver — port of `cli/src/browser/selector-resolver.ts`.
//!
//! Ref → selector resolution, selector parsing, and strictness errors.

use crate::browser::errors::{unknown_ref_error, SelectorError};
use crate::browser::types::BrowserConfig;
use serde_json::Value;

/// `ParsedSelectorEngine` — "css" | "xpath" | "text".
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedSelectorEngine {
    Css,
    Xpath,
    Text,
}

/// `ParsedSelector`.
#[derive(Debug, Clone)]
pub struct ParsedSelector {
    pub engine: ParsedSelectorEngine,
    pub body: String,
}

/// `resolveTarget(input, config)` — ref lookup, then direct selector.
pub fn resolve_target(
    input: Option<&str>,
    config: &BrowserConfig,
) -> Result<ResolvedTarget, SelectorError> {
    let raw = input.map(str::trim).unwrap_or("");
    if raw.is_empty() {
        return Err(SelectorError::new(
            "Expected ref, selector, or target.",
            "missing_input",
            None,
        ));
    }

    // If it looks like a ref (e.g., b1, i2, a3) and the config has refs, resolve it
    if looks_like_ref(raw) {
        let r#ref = raw.to_lowercase(); // refs are case-insensitive (a1, A1, etc.)
        if let Some(refs) = &config.refs {
            if let Some(Value::String(selector)) = refs.get(&r#ref) {
                return Ok(ResolvedTarget {
                    original: raw.to_string(),
                    source: "ref",
                    selector: selector.clone(),
                    r#ref: Some(r#ref),
                    parsed: parse_selector(selector),
                });
            }
        }
        // It looks like a ref but no matching entry → error with context
        return Err(unknown_ref_error(&r#ref));
    }

    // It's a direct selector
    Ok(ResolvedTarget {
        original: raw.to_string(),
        source: "selector",
        selector: raw.to_string(),
        r#ref: None,
        parsed: parse_selector(raw),
    })
}

/// `ResolvedTarget` — `{ original, source, selector, ref?, parsed }`.
#[derive(Debug, Clone)]
pub struct ResolvedTarget {
    pub original: String,
    pub source: &'static str,
    pub selector: String,
    pub r#ref: Option<String>,
    pub parsed: ParsedSelector,
}

/// `legacySelectorFor(args, config)` — matches the old `selectorFor()` behavior.
pub fn legacy_selector_for(
    args: &serde_json::Map<String, serde_json::Value>,
    config: &BrowserConfig,
) -> Result<String, SelectorError> {
    let selector = string_arg(args, "selector");
    if let Some(s) = selector {
        return Ok(s);
    }

    let target = string_arg(args, "target");
    let r#ref = string_arg(args, "ref").or_else(|| {
        target
            .as_ref()
            .filter(|t| looks_like_ref(t))
            .map(|t| t.to_lowercase())
    });

    if let Some(r#ref) = r#ref {
        let resolved = config
            .refs
            .as_ref()
            .and_then(|m| m.get(&r#ref))
            .and_then(serde_json::Value::as_str);
        match resolved {
            Some(selector) => return Ok(selector.to_string()),
            None => return Err(unknown_ref_error(&r#ref)),
        }
    }

    if let Some(target) = target {
        return Ok(target);
    }

    Err(SelectorError::new(
        "Expected ref, selector, or target.",
        "missing_input",
        None,
    ))
}

/// `parseSelector(raw)` — engine + body.
pub fn parse_selector(raw: &str) -> ParsedSelector {
    if let Some(body) = raw.strip_prefix("text=") {
        return ParsedSelector {
            engine: ParsedSelectorEngine::Text,
            body: body.to_string(),
        };
    }
    if let Some(body) = raw.strip_prefix("xpath=") {
        return ParsedSelector {
            engine: ParsedSelectorEngine::Xpath,
            body: body.to_string(),
        };
    }
    ParsedSelector {
        engine: ParsedSelectorEngine::Css,
        body: raw.to_string(),
    }
}

// ── Internal ────────────────────────────────────────────────────────

/// `/^[a-z]\d+$/i` — a ref-shaped token.
fn looks_like_ref(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let first = bytes[0];
    if !first.is_ascii_alphabetic() {
        return false;
    }
    bytes[1..].iter().all(|b| b.is_ascii_digit()) && bytes.len() > 1
}

fn string_arg(args: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    match args.get(key) {
        Some(serde_json::Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_config() -> BrowserConfig {
        let raw = json!({
            "version": 2,
            "connection": {"protocol": "cdp", "browserKind": "chrome", "endpoint": "http://127.0.0.1:9222"},
            "refs": {"b1": "#btn-submit", "i2": "input[data-testid='email']"}
        });
        crate::browser::browser_state::parse_browser_config(&raw).unwrap()
    }

    #[test]
    fn ref_resolves_to_stored_selector() {
        let config = base_config();
        let result = resolve_target(Some("b1"), &config).unwrap();
        assert_eq!(result.source, "ref");
        assert_eq!(result.selector, "#btn-submit");
        assert_eq!(result.r#ref.as_deref(), Some("b1"));
    }

    #[test]
    fn unknown_ref_with_refs_present_errors() {
        // Refs exist but the requested key is not among them.
        let config = base_config();
        let err = resolve_target(Some("b9"), &config).unwrap_err();
        assert_eq!(err.error.code, "unknown_ref");
        assert_eq!(err.selector.as_deref(), Some("b9"));
    }

    #[test]
    fn ref_shaped_token_detection() {
        assert!(super::looks_like_ref("b1"));
        assert!(super::looks_like_ref("A12"));
        assert!(!super::looks_like_ref(""));
        // Non-alphabetic first byte is not ref-shaped.
        assert!(!super::looks_like_ref("1b"));
        assert!(!super::looks_like_ref("#b1"));
    }

    #[test]
    fn ref_is_case_insensitive() {
        let config = base_config();
        let result = resolve_target(Some("B1"), &config).unwrap();
        assert_eq!(result.source, "ref");
        assert_eq!(result.selector, "#btn-submit");
    }

    #[test]
    fn unknown_ref_throws_unknown_ref_error() {
        let config = base_config();
        let err = resolve_target(Some("b99"), &config).unwrap_err();
        assert_eq!(err.error.code, "unknown_ref");
        assert!(err.error.message.contains("\"b99\""));
        assert!(err.error.message.contains("snapshot"));
    }

    #[test]
    fn selector_passes_through_directly() {
        let config = base_config();
        let result = resolve_target(Some("#my-id"), &config).unwrap();
        assert_eq!(result.source, "selector");
        assert_eq!(result.selector, "#my-id");
    }

    #[test]
    fn text_selector_parsed_as_text_engine() {
        let config = base_config();
        let result = resolve_target(Some("text=Submit"), &config).unwrap();
        assert_eq!(result.parsed.engine, ParsedSelectorEngine::Text);
        assert_eq!(result.parsed.body, "Submit");
    }

    #[test]
    fn xpath_selector_parsed_as_xpath_engine() {
        let config = base_config();
        let result = resolve_target(Some("xpath=//button"), &config).unwrap();
        assert_eq!(result.parsed.engine, ParsedSelectorEngine::Xpath);
    }

    #[test]
    fn html_selector_parsed_as_css_engine() {
        let config = base_config();
        let result = resolve_target(Some(".btn-primary"), &config).unwrap();
        assert_eq!(result.parsed.engine, ParsedSelectorEngine::Css);
    }

    #[test]
    fn empty_input_throws() {
        let config = base_config();
        assert!(resolve_target(Some(""), &config).is_err());
        assert!(resolve_target(None, &config).is_err());
    }

    #[test]
    fn parse_selector_css() {
        assert_eq!(parse_selector("#foo").engine, ParsedSelectorEngine::Css);
        assert_eq!(parse_selector(".bar").engine, ParsedSelectorEngine::Css);
        assert_eq!(
            parse_selector("div > span").engine,
            ParsedSelectorEngine::Css
        );
    }

    #[test]
    fn parse_selector_text() {
        let p = parse_selector("text=Click me");
        assert_eq!(p.engine, ParsedSelectorEngine::Text);
        assert_eq!(p.body, "Click me");
    }

    #[test]
    fn parse_selector_xpath() {
        let p = parse_selector("xpath=//div");
        assert_eq!(p.engine, ParsedSelectorEngine::Xpath);
        assert_eq!(p.body, "//div");
    }

    #[test]
    fn legacy_selector_for_prefers_selector() {
        let config = base_config();
        let args = serde_json::Map::new();
        // selector arg wins
        let mut args = args;
        args.insert("selector".into(), json!("#direct"));
        args.insert("ref".into(), json!("b1"));
        assert_eq!(legacy_selector_for(&args, &config).unwrap(), "#direct");
    }

    #[test]
    fn legacy_selector_for_ref_resolves() {
        let config = base_config();
        let mut args = serde_json::Map::new();
        args.insert("ref".into(), json!("b1"));
        assert_eq!(legacy_selector_for(&args, &config).unwrap(), "#btn-submit");
    }

    #[test]
    fn legacy_selector_for_target_looks_like_ref() {
        let config = base_config();
        let mut args = serde_json::Map::new();
        args.insert("target".into(), json!("i2"));
        assert_eq!(
            legacy_selector_for(&args, &config).unwrap(),
            "input[data-testid='email']"
        );
    }

    #[test]
    fn legacy_selector_for_unknown_ref_throws() {
        let config = base_config();
        let mut args = serde_json::Map::new();
        args.insert("ref".into(), json!("z9"));
        let err = legacy_selector_for(&args, &config).unwrap_err();
        assert_eq!(err.error.code, "unknown_ref");
    }

    #[test]
    fn legacy_selector_for_target_passthrough() {
        let config = base_config();
        let mut args = serde_json::Map::new();
        args.insert("target".into(), json!("button.foo"));
        assert_eq!(legacy_selector_for(&args, &config).unwrap(), "button.foo");
    }

    #[test]
    fn legacy_selector_for_missing_input_throws() {
        let config = base_config();
        let args = serde_json::Map::new();
        let err = legacy_selector_for(&args, &config).unwrap_err();
        assert_eq!(err.error.message, "Expected ref, selector, or target.");
    }
}
