//! Match typography, never paraphrases or a different participant's words.
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
