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
    /// Rendering markers a participant's answer carries and a quote drops while
    /// copying its wording: emphasis and code spans (`**一定**`, `*一定*`,
    /// `__一定__`, `` `一定` ``). Quoting the words behind such markers is the
    /// same excerpt, so the markers never decide a match.
    ///
    /// Single `_` and `~` stay: they delimit identifiers (`max_tokens`) and
    /// ranges (`3~5 年`), where dropping them would conflate different text.
    fn is_markup(c: char, previous: Option<char>, next: Option<char>) -> bool {
        match c {
            '*' | '`' => true,
            '_' | '~' => previous == Some(c) || next == Some(c),
            _ => false,
        }
    }
    // A copied quotation can render $A$ or $n-2$ without the delimiters.
    // Keep currency, escaped dollar signs, and display math delimiters literal.
    fn inline_math_end(chars: &[char], start: usize) -> Option<usize> {
        if matches!(
            start.checked_sub(1).and_then(|index| chars.get(index)),
            Some('\\' | '$')
        ) {
            return None;
        }
        let end = match chars.get(start + 1..)? {
            [name, '$', ..] if name.is_alphabetic() => start + 2,
            [name, '+' | '-', digit, rest @ ..]
                if name.is_alphabetic() && digit.is_ascii_digit() =>
            {
                start + 4 + rest.iter().position(|c| !c.is_ascii_digit())?
            }
            _ => return None,
        };
        (chars.get(end) == Some(&'$') && chars.get(end + 1) != Some(&'$')).then_some(end)
    }
    fn normalize(text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut result = String::new();
        let mut whitespace = false;
        let mut inline_math_end_index = None;
        let chinese = |c: char| matches!(c, '\u{2e80}'..='\u{9fff}' | '\u{f900}'..='\u{faff}');
        for (index, &c) in chars.iter().enumerate() {
            if c.is_whitespace() {
                whitespace = true;
                continue;
            }
            let markup = is_markup(
                c,
                index.checked_sub(1).map(|previous| chars[previous]),
                chars.get(index + 1).copied(),
            );
            if markup {
                continue;
            }
            if c == '$' {
                if inline_math_end_index == Some(index) {
                    inline_math_end_index = None;
                    continue;
                }
                inline_math_end_index = inline_math_end(&chars, index);
                if inline_math_end_index.is_some() {
                    continue;
                }
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
    fn a_rendered_math_variable_matches_its_source_without_losing_meaning() {
        assert!(contains(
            "and $A$ replies by giving $B$ the number",
            "A replies by giving B the number"
        ));
        assert!(!contains(
            "and $A$ replies by giving $B$ the number",
            "B replies by giving A the number"
        ));
        assert!(!contains("Pay $5 and keep $6", "Pay 5 and keep 6"));
        assert!(!contains(r"Write \$A$ literally", "Write A literally"));
        assert!(!contains("Figure $$A$$", "Figure A"));
    }
    #[test]
    fn rendered_inline_arithmetic_keeps_the_index_and_operator() {
        let source = "for all $i$ between 1 and $n-2$ inclusive, if the $i$th step is upward, the $i+1$st step must be rightward";
        let excerpt = "for all i between 1 and n-2 inclusive, if the ith step is upward, the i+1st step must be rightward";
        assert!(contains(source, excerpt));
        assert!(!contains(source, &excerpt.replace("n-2", "n-1")));
        assert!(!contains(source, &excerpt.replace("i+1", "i-1")));
        assert!(!contains("Use $$n-2$$ here", "Use n-2 here"));
        assert!(!contains(r"Use \$n-2$ here", "Use n-2 here"));
        assert!(!contains("Pay $5-2$ here", "Pay 5-2 here"));
    }
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

    /// The live failure this guards: a moderator quoted
    /// `存在性风险论：概率太低，…` from a bullet that reads
    /// `- **存在性风险论**：概率太低，…` and the exact-excerpt check rejected it.
    #[test]
    fn tolerates_emphasis_and_code_markers_without_changing_words() {
        assert!(contains(
            "- **存在性风险论**：概率太低，且依赖于认知冲击已经发生（人类失去控制）",
            "存在性风险论：概率太低，且依赖于认知冲击已经发生（人类失去控制）"
        ));
        assert!(contains("**重点**：不可逆", "*重点*：不可逆"));
        assert!(contains("__bold__ text", "bold text"));
        assert!(contains(
            "这是 `max_tokens` 的上限",
            "这是 max_tokens 的上限"
        ));
        assert!(contains("~~struck~~ out", "struck out"));
        assert!(contains(
            "**「保留数据」** 是他的话",
            "「保留数据」 是他的话"
        ));
    }

    /// Markers are typography, not a licence to paraphrase.
    #[test]
    fn markers_never_make_different_words_match() {
        assert!(!contains(
            "**存在性风险论**：概率太低",
            "存在性风险论：概率很高"
        ));
        assert!(!contains("**Use SQLite.**", "Use files."));
    }

    /// A single underscore or tilde is content: identifiers and ranges keep
    /// their exact spelling.
    #[test]
    fn a_single_underscore_or_tilde_is_content() {
        assert!(contains("`max_tokens` 默认 4096", "max_tokens 默认 4096"));
        assert!(!contains("max_tokens", "maxtokens"));
        assert!(!contains("约 3~5 年", "约 35 年"));
    }
}
