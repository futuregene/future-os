//! Native dialogs whose text has to be in the language the window is showing.
//!
//! The update flow carries a language tag on the invoke that starts it; the quit
//! confirmation is raised from a window/run event instead, with no invoke of its
//! own. So the window registers the language it resolved (see
//! `set_ui_language`) and this module reads it when a dialog is built. The
//! strings come from the same two bundled catalogues `update_dialog_text` reads,
//! so native copy stays translated in exactly one place.
//!
//! A dialog raised before the webview ever registered a language — a window that
//! never loaded — reads English, which is what these dialogs said before they
//! were translated at all.

use std::sync::{LazyLock, Mutex, PoisonError};

/// Which bundled catalogue a native dialog reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Locale {
    En,
    Zh,
}

/// The language the window is showing, as the frontend resolved it.
static UI_LANGUAGE: Mutex<Option<String>> = Mutex::new(None);

/// Serializes the two tests that read the registry back: they each set a tag and
/// assert what a dialog would see, which is only meaningful without another test
/// setting one in between.
#[cfg(test)]
pub(crate) fn language_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn set_language(tag: &str) {
    let tag = tag.trim();
    if tag.is_empty() {
        return;
    }
    // A dialog raised while another thread holds this lock is still better than
    // a quit that panics: the value is one string and poisoning carries nothing.
    *UI_LANGUAGE.lock().unwrap_or_else(PoisonError::into_inner) = Some(tag.to_owned());
}

pub(crate) fn locale() -> Locale {
    let tag = UI_LANGUAGE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    locale_of(tag.as_deref())
}

/// The rule `update_dialog_text` applies: a `zh*` tag reads Chinese and every
/// other tag — including none at all — reads English.
pub(crate) fn locale_of(tag: Option<&str>) -> Locale {
    match tag {
        Some(value) if value.starts_with("zh") => Locale::Zh,
        _ => Locale::En,
    }
}

static EN: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../src/i18n/locales/en/settings.json"))
        .expect("bundled English settings translations must be valid JSON")
});

static ZH: LazyLock<serde_json::Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../src/i18n/locales/zh/settings.json"))
        .expect("bundled Chinese settings translations must be valid JSON")
});

/// Every string the quit confirmation asks for. The test below walks this list,
/// so a key that never made it into one catalogue fails before a user sees it.
#[cfg(test)]
const QUIT_KEYS: [&str; 10] = [
    "title",
    "confirm",
    "defer",
    "quitAnyway",
    "streamingOne",
    "streamingMany",
    "otherWork",
    "unknown",
    "costOwned",
    "costAttached",
];

fn quit_string(locale: Locale, key: &str) -> String {
    let catalog = match locale {
        Locale::Zh => &*ZH,
        Locale::En => &*EN,
    };
    catalog["quit"][key]
        .as_str()
        .unwrap_or_else(|| panic!("native quit dialog string `quit.{key}` is missing"))
        .to_owned()
}

/// What the quit decision counted, as the confirmation has to say it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuitWork {
    /// Conversations with a reply in flight. Never zero: see
    /// `agent_supervisor::quit_work`.
    Streaming(usize),
    /// No reply in flight, but the Agent is doing something else — queued
    /// messages, an approval waiting, compaction, an in-flight command or
    /// background work.
    Other,
    /// The count could not be read at all.
    Unknown,
}

pub(crate) struct QuitConfirmation {
    pub title: String,
    pub message: String,
    pub confirm: String,
    pub defer: String,
}

/// The whole quit dialog in one place: what is still running, what quitting
/// costs, and the question.
pub(crate) fn quit_confirmation(
    locale: Locale,
    work: QuitWork,
    owns_agent: bool,
) -> QuitConfirmation {
    let text = |key: &str| quit_string(locale, key);
    let fact = match work {
        QuitWork::Streaming(1) => text("streamingOne"),
        QuitWork::Streaming(count) => {
            text("streamingMany").replace("{{count}}", &count.to_string())
        }
        QuitWork::Other => text("otherWork"),
        QuitWork::Unknown => text("unknown"),
    };
    let cost = text(if owns_agent {
        "costOwned"
    } else {
        "costAttached"
    });
    // Chinese sentences end in 。 and read worse with a space between them.
    let separator = match locale {
        Locale::Zh => "",
        Locale::En => " ",
    };
    QuitConfirmation {
        title: text("title"),
        message: format!("{fact}{separator}{cost}{separator}{}", text("quitAnyway")),
        confirm: text("confirm"),
        defer: text("defer"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_quit_string_is_translated_in_both_languages() {
        for key in QUIT_KEYS {
            let english = quit_string(Locale::En, key);
            let chinese = quit_string(Locale::Zh, key);
            assert!(!english.is_empty(), "quit.{key} has no English text");
            assert!(!chinese.is_empty(), "quit.{key} has no Chinese text");
            assert_ne!(chinese, english, "quit.{key} was not translated");
        }
    }

    #[test]
    fn a_language_tag_picks_the_bundle_the_window_shows() {
        assert_eq!(locale_of(Some("zh-CN")), Locale::Zh);
        assert_eq!(locale_of(Some("zh-TW")), Locale::Zh);
        assert_eq!(locale_of(Some("en-US")), Locale::En);
        assert_eq!(locale_of(Some("de")), Locale::En);
        assert_eq!(locale_of(None), Locale::En);
    }

    #[test]
    fn the_registered_language_is_what_a_dialog_reads() {
        let _serialized = language_test_guard();
        set_language("zh-CN");
        assert_eq!(locale(), Locale::Zh);
        set_language("en-US");
        assert_eq!(locale(), Locale::En);
        set_language("   ");
        assert_eq!(
            locale(),
            Locale::En,
            "blank registrations keep the last one"
        );
    }

    #[test]
    fn the_confirmation_counts_the_conversations_that_are_still_generating() {
        let english = quit_confirmation(Locale::En, QuitWork::Streaming(2), true);
        assert!(
            english
                .message
                .contains("2 conversations are still generating replies"),
            "{}",
            english.message
        );
        assert!(
            english.message.contains("CLI"),
            "the blast radius is named: {}",
            english.message
        );
        assert!(
            english
                .message
                .ends_with(&quit_string(Locale::En, "quitAnyway")),
            "the question closes the message: {}",
            english.message
        );
        assert_eq!(english.confirm, "Force quit");
        assert_eq!(english.defer, "Keep running");

        let chinese = quit_confirmation(Locale::Zh, QuitWork::Streaming(2), true);
        assert!(
            chinese.message.contains("还有 2 个会话正在生成回复。"),
            "{}",
            chinese.message
        );
        assert!(
            !chinese.message.contains("{{count}}"),
            "the count is interpolated: {}",
            chinese.message
        );
    }

    #[test]
    fn one_conversation_is_not_pluralised_in_english() {
        let one = quit_confirmation(Locale::En, QuitWork::Streaming(1), false);
        assert!(
            one.message
                .starts_with("1 conversation is still generating a reply."),
            "{}",
            one.message
        );
        assert!(
            !one.message.contains("conversations are"),
            "{}",
            one.message
        );
    }

    #[test]
    fn a_count_without_a_reply_in_flight_says_what_is_running_instead() {
        let other = quit_confirmation(Locale::En, QuitWork::Other, true);
        assert!(
            other
                .message
                .contains("No conversation is generating a reply"),
            "{}",
            other.message
        );
        assert!(
            !other.message.contains("{{count}}"),
            "no placeholder survives: {}",
            other.message
        );
        assert_ne!(
            other.message,
            quit_confirmation(Locale::En, QuitWork::Streaming(1), true).message,
            "the two cases must not read the same"
        );
    }

    #[test]
    fn an_unreadable_count_says_it_cannot_check() {
        let unknown = quit_confirmation(Locale::Zh, QuitWork::Unknown, false);
        assert!(
            unknown.message.contains("暂时无法确认是否还有会话在生成。"),
            "{}",
            unknown.message
        );
        assert!(
            unknown.message.contains("外部 Agent"),
            "an attached Agent still promises the other clients keep running: {}",
            unknown.message
        );
    }

    #[test]
    fn the_cost_sentence_follows_who_owns_the_agent() {
        let owned = quit_confirmation(Locale::En, QuitWork::Streaming(2), true);
        let attached = quit_confirmation(Locale::En, QuitWork::Streaming(2), false);
        assert!(
            owned.message.contains("every run on this machine"),
            "{}",
            owned.message
        );
        assert!(
            attached.message.contains("runs this Desktop submitted"),
            "{}",
            attached.message
        );
        assert_ne!(owned.message, attached.message);
    }
}
