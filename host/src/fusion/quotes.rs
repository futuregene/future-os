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
            if whitespace
                && !result.is_empty()
                && !(result.chars().last().is_some_and(chinese) && chinese(c))
            {
                result.push(' ');
            }
            whitespace = false;
            result.push(match c {
                '\'' | '"' | '‘' | '’' | '“' | '”' | '＇' | '＂' => '"',
                _ => c,
            });
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
        assert!(!contains("Do not delete data.", "Do delete data."));
        assert!(!contains("Use SQLite.", "Use files."));
        assert!(!contains("now here", "nowhere"));
        assert!(!contains("anything", " \n\u{3000}"));
    }
}
