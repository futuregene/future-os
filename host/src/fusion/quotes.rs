//! Match typography and resolve references to frozen source text.
use anyhow::{ensure, Context, Result};
use serde_json::{Map, Value};

pub(super) fn select_text<'a>(source: &'a Value, path: &str) -> Option<&'a str> {
    if !path.starts_with('/')
        || matches!(
            path.rsplit('/').next(),
            Some("id" | "participant" | "source_id" | "source_phase" | "decision" | "assessment")
        )
    {
        return None;
    }
    source
        .pointer(path)?
        .as_str()
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn expand<'a>(
    row: &mut Map<String, Value>,
    path_key: &str,
    quote_key: &str,
    resolve: impl FnOnce(&str) -> Option<&'a str>,
) -> Result<()> {
    let Some(path) = row.remove(path_key) else {
        return Ok(());
    };
    ensure!(
        !row.contains_key(quote_key),
        "Supply either {path_key} or {quote_key}"
    );
    let path = path.as_str().context("Source path must be a string")?;
    let quote = resolve(path)
        .with_context(|| format!("{path_key} {path:?} does not select visible source text"))?;
    row.insert(quote_key.into(), Value::String(quote.into()));
    Ok(())
}

pub(super) fn contains_report(source: &str, excerpt: &str) -> bool {
    fn matches(value: &Value, excerpt: &str) -> bool {
        match value {
            Value::String(text) => contains(text, excerpt),
            Value::Array(items) => items.iter().any(|item| matches(item, excerpt)),
            Value::Object(fields) => fields.values().any(|value| matches(value, excerpt)),
            _ => false,
        }
    }
    contains(source, excerpt)
        || future_fusion::parse_json::<Value>(source).is_ok_and(|value| matches(&value, excerpt))
}

pub(super) fn contains(source: &str, excerpt: &str) -> bool {
    fn normalize(text: &str) -> String {
        let mut result = String::new();
        let mut whitespace = false;
        let chinese = |c: char| matches!(c, '\u{2e80}'..='\u{9fff}' | '\u{f900}'..='\u{faff}');
        for c in text.chars() {
            if c.is_whitespace() {
                whitespace = true;
                continue;
            }
            let c = match c {
                '\'' | '"' | '‘' | '’' | '“' | '”' | '＇' | '＂' | '「' | '」' | '『' | '』' => {
                    '"'
                }
                _ => c,
            };
            if whitespace
                && result.chars().next_back().is_some_and(|previous| {
                    !(chinese(previous) && (chinese(c) || c == '"')
                        || previous == '"' && chinese(c))
                })
            {
                result.push(' ');
            }
            whitespace = false;
            result.push(c);
        }
        result.trim().to_owned()
    }
    let excerpt = normalize(excerpt);
    !excerpt.is_empty() && normalize(source).contains(&excerpt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tolerates_quotes_and_whitespace_without_changing_words() {
        assert!(contains("不要在电台上“钓鱼”", "不要在电台上'钓鱼'"));
        assert!(contains(
            "Keep\r\n existing\u{3000}data.",
            "Keep existing data."
        ));
        assert!(contains(
            "它说\"本质仍是工具\"是断言而非论证",
            "「本质仍是工具」 是断言而非论证"
        ));
        assert!(contains("他说『保留数据』。", "他说 \"保留数据\"。"));
        assert!(!contains("「不执行」是决定", "\"执行\"是决定"));
        assert!(!contains("Do not delete data.", "Do delete data."));
        assert!(!contains("Use SQLite.", "Use files."));
        assert!(!contains("now here", "nowhere"));
        assert!(!contains("anything", " \n\u{3000}"));
    }
}
