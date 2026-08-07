import i18n from "../i18n";

// Tests assert against the canonical English wording, so pin the test locale to
// English regardless of the app's default (Chinese) language. Resources are
// bundled inline, so changeLanguage applies synchronously.
void i18n.changeLanguage("en");
