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
//! The probe reads the login session on macOS (a plain C call) and the display
//! environment on Linux (the sources GTK itself consults). Windows has no
//! equivalent: a GUI process started from a service context still reaches the
//! desktop, so there is nothing to pre-empt there.

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
     last startup step. Sign in at this machine's own screen — or through a Screen Sharing/VNC \
     session — and open FutureOS there. On a server with no display, run `future host` instead: it \
     serves remote devices without a window.";

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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn current() -> SessionKind {
    // Windows: a GUI process started from SSH or a service still reaches the
    // desktop, so no session here needs pre-empting.
    SessionKind::Graphical
}

#[cfg(target_os = "linux")]
fn current() -> SessionKind {
    // libwayland falls back to this socket when `WAYLAND_DISPLAY` never reached
    // the process — an import-less systemd unit, for instance — while still being
    // a launch a window can open in.
    let wayland_default = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .map(|directory| directory.join("wayland-0"))
        .is_some_and(|socket| socket.exists());
    linux_session(Displays {
        x11: std::env::var_os("DISPLAY").as_deref(),
        wayland: std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        broadway: std::env::var_os("BROADWAY_DISPLAY").as_deref(),
        wayland_default,
    })
}

/// What the environment offers GTK as a display.
///
/// Gathered into a value so Linux's answer can be exercised on a host that is not
/// Linux — the platform this branch exists for is exactly the one a developer's
/// machine cannot compile in isolation.
#[cfg(any(target_os = "linux", test))]
#[derive(Clone, Copy, Default)]
struct Displays<'a> {
    /// `DISPLAY`: an X11 server, an XWayland one included.
    x11: Option<&'a std::ffi::OsStr>,
    /// `WAYLAND_DISPLAY`: a Wayland compositor.
    wayland: Option<&'a std::ffi::OsStr>,
    /// `BROADWAY_DISPLAY`: GTK's HTML backend.
    broadway: Option<&'a std::ffi::OsStr>,
    /// Whether `$XDG_RUNTIME_DIR/wayland-0` is there, which is what libwayland
    /// connects to when `WAYLAND_DISPLAY` was dropped on the way to this process.
    wayland_default: bool,
}

/// Linux's answer: a display is present when any of GTK's own ways to find one is.
///
/// These are the same sources `gtk_init` consults, except that asking here can
/// answer "no" — which is the whole point: `gtk_init` answers by aborting the
/// process inside tao, after the window build has already begun. So the refusal has
/// to come before that build, and it must only speak when nothing here could have
/// worked, or a launch that would have opened a window would be turned away.
#[cfg(any(target_os = "linux", test))]
fn linux_session(displays: Displays<'_>) -> SessionKind {
    // An empty value is what a login shell leaves behind, not a display.
    let set = |value: Option<&std::ffi::OsStr>| value.is_some_and(|value| !value.is_empty());
    if set(displays.x11)
        || set(displays.wayland)
        || set(displays.broadway)
        || displays.wayland_default
    {
        SessionKind::Graphical
    } else {
        SessionKind::Background
    }
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

    /// Linux decides from the display environment, and this is the case CI hit: a
    /// runner with none of it set used to reach GTK, which aborts the process (exit
    /// 101), instead of refusing with the message above. Every source is asserted
    /// separately, because refusing a launch that one of them would have served is
    /// the failure mode of a probe that guesses too much.
    #[test]
    fn a_linux_launch_keeps_every_display_gtk_could_find() {
        use std::ffi::OsStr;
        let displays = |x11: Option<&'static OsStr>, wayland, broadway| Displays {
            x11,
            wayland,
            broadway,
            wayland_default: false,
        };
        assert_eq!(
            linux_session(displays(Some(OsStr::new(":0")), None, None)),
            SessionKind::Graphical
        );
        assert_eq!(
            linux_session(displays(None, Some(OsStr::new("wayland-0")), None)),
            SessionKind::Graphical
        );
        assert_eq!(
            linux_session(displays(None, None, Some(OsStr::new("localhost:8080")))),
            SessionKind::Graphical
        );
        // The socket libwayland falls back to when the variable was lost: still a
        // display a window can open in.
        assert_eq!(
            linux_session(Displays {
                wayland_default: true,
                ..Displays::default()
            }),
            SessionKind::Graphical
        );
        // An empty variable is what a login shell leaves behind, not a display,
        // and reading it as one would turn the abort back into a silent wait.
        assert_eq!(
            linux_session(displays(
                Some(OsStr::new("")),
                Some(OsStr::new("")),
                Some(OsStr::new(""))
            )),
            SessionKind::Background
        );
        assert_eq!(linux_session(Displays::default()), SessionKind::Background);
    }
}
