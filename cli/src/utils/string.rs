//! String helpers — port of `cli/src/utils/string.ts`.

/// `url.replace(/\/+$/, "")` — strip all trailing slashes.
pub fn trim_trailing_slash(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// Port of `escapeXml` — must match JS replace semantics: each pass replaces
/// the current character only (later passes never re-touch earlier output).
pub fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_trailing_slash_behavior() {
        assert_eq!(
            trim_trailing_slash("https://future-os.cn"),
            "https://future-os.cn"
        );
        assert_eq!(
            trim_trailing_slash("https://future-os.cn/"),
            "https://future-os.cn"
        );
        assert_eq!(
            trim_trailing_slash("https://future-os.cn///"),
            "https://future-os.cn"
        );
        assert_eq!(trim_trailing_slash(""), "");
        assert_eq!(trim_trailing_slash("/"), "");
    }

    #[test]
    fn escape_xml_behavior() {
        assert_eq!(
            escape_xml("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
        assert_eq!(escape_xml("no specials"), "no specials");
        assert_eq!(escape_xml("&amp;"), "&amp;amp;");
    }
}
