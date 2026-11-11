//! Whether this launch can reach the machine's display.
//!
//! A GUI launch from a background bootstrap namespace — an SSH login, a daemon
//! context, or a Mac sitting at the login window — never gets a WindowServer
//! connection. AppKit then never posts `applicationDidFinishLaunching`, tao
//! never sends `RunEvent::Ready`, and the launch waits in the event loop
//! forever with no window: the watchdog reports "stalled at step 'event loop'",
//! which reads like a deadlock although the launch was never viable. Asking the
//! session question before entering the loop turns that into a refusal the user
//! can act on, with an exit code a script can see.
//!
//! The probe is a plain C call on macOS and a no-op elsewhere: Windows reaches
//! its desktop from a service context, and on Linux tao fails loudly while
//! creating the window, so neither has this silent stall to pre-empt.

/// Where a launch is running from, as far as the display is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionKind {
    /// Attached to a signed-in desktop session: a window can open.
    Graphical,
    /// No session with a display: the event loop would never become ready.
    Background,
}

/// What a refused launch is told, in the host log and on stderr. It names the
/// evidence and both ways forward, because whoever reads it launched from a
/// terminal and needs to know where the window has to be started instead.
pub(crate) const NO_GRAPHICAL_SESSION: &str =
    "FutureOS needs a graphical session: this process is not \
     attached to a signed-in desktop, so the window could never open — a launch from SSH or another \
     background session would wait in the event loop forever, with no window and nothing after its \
     last startup step. Sign in on this Mac's own screen — or a Screen Sharing/VNC session — and \
     open FutureOS there. On a server with no display, run `future host` instead: it serves remote \
     devices without a window.";

/// The seam that makes both outcomes exercisable on any machine: a test (or a
/// CI job with no display) forces the probe instead of inheriting it from the
/// environment it happens to run in.
const SESSION_OVERRIDE: &str = "FUTURE_DESKTOP_SESSION";

/// Detect the session this launch runs in, honouring [`SESSION_OVERRIDE`].
pub(crate) fn detect() -> SessionKind {
    if let Some(forced) = std::env::var(SESSION_OVERRIDE)
        .ok()
        .as_deref()
        .and_then(parse)
    {
        return forced;
    }
    current()
}

/// The refusal decision, split from the probe so the message and the two
/// outcomes are testable without a desktop.
pub(crate) fn decide(session: SessionKind) -> Result<(), String> {
    match session {
        SessionKind::Graphical => Ok(()),
        SessionKind::Background => Err(NO_GRAPHICAL_SESSION.to_string()),
    }
}

/// [`decide`] against what this launch actually is.
pub(crate) fn require() -> Result<(), String> {
    decide(detect())
}

/// `Ok(None)` for an absent or unrecognized override: an unknown value must not
/// silently pick a session kind.
fn parse(value: &str) -> Option<SessionKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "graphical" | "aqua" => Some(SessionKind::Graphical),
        "background" | "none" => Some(SessionKind::Background),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn current() -> SessionKind {
    use std::ffi::c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        /// The current login session's dictionary, or null when this process
        /// has no session with a display (SSH, launchd background context, or
        /// nobody signed in). The copy is owned by the caller.
        fn CGSessionCopyCurrentDictionary() -> *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(value: *const c_void);
    }

    // SAFETY: plain C calls with no aliasing and no ownership beyond the
    // dictionary, which arrives +1 and is released here.
    unsafe {
        let session = CGSessionCopyCurrentDictionary();
        if session.is_null() {
            return SessionKind::Background;
        }
        CFRelease(session);
    }
    SessionKind::Graphical
}

#[cfg(not(target_os = "macos"))]
fn current() -> SessionKind {
    // Windows: a GUI process started from SSH or a service still reaches the
    // desktop. Linux: window creation fails with an error instead of stalling.
    SessionKind::Graphical
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_graphical_session_proceeds() {
        assert!(decide(SessionKind::Graphical).is_ok());
    }

    #[test]
    fn a_background_session_is_refused_with_both_ways_forward() {
        let refusal = decide(SessionKind::Background).unwrap_err();
        assert!(refusal.contains("needs a graphical session"), "{refusal}");
        // The window has to be started somewhere else, and a server has an
        // entry point that needs no window at all.
        assert!(refusal.contains("screen"), "{refusal}");
        assert!(refusal.contains("future host"), "{refusal}");
    }

    #[test]
    fn only_known_override_values_are_taken() {
        assert_eq!(parse("background"), Some(SessionKind::Background));
        assert_eq!(parse(" Graphical "), Some(SessionKind::Graphical));
        assert_eq!(parse("aqua"), Some(SessionKind::Graphical));
        assert_eq!(parse("headless"), None);
        assert_eq!(parse(""), None);
    }

    /// The override is what makes the refusal reproducible off a desktop, so
    /// the same value a test writes is the one `detect` reads.
    #[test]
    fn the_override_steers_detection() {
        if std::env::var(SESSION_OVERRIDE).is_ok() {
            return;
        }
        std::env::set_var(SESSION_OVERRIDE, "background");
        let detected = detect();
        std::env::remove_var(SESSION_OVERRIDE);
        assert_eq!(detected, SessionKind::Background);
    }
}
