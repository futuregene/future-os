//! Mouse input: SGR 1006 escape sequences in, [`MouseEvent`] out.
//!
//! The workbench is the first surface in this crate that is *clickable*, so the
//! parsing lives here rather than in the terminal backend: the backend owns
//! bytes and raw mode, this module owns the protocol. Two reasons:
//!
//! * SGR 1006 is a small, fully specified grammar, and it is testable as such —
//!   no pty needed, no timing. `?1006h` makes the terminal report `\x1b[<b;x;yM`
//!   (press/motion) and `…m` (release), with 1-based coordinates.
//! * Only the workbench enables it: the mode is written by the terminal that
//!   was asked for it (`Terminal::set_mouse_mode`), so a caller with no mouse
//!   targets never sees one of these reports.

/// What the pointer did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    /// Motion with a button held.
    Drag,
    /// Motion with no button held (`?1003h` hover tracking).
    Move,
    WheelUp,
    WheelDown,
}

/// One decoded pointer event. `x`/`y` are 0-based cell coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub x: u16,
    pub y: u16,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl MouseEvent {
    /// Is a button held during this event? Drags and releases carry their own
    /// bit; a press always does.
    pub fn is_left_press(&self) -> bool {
        self.kind == MouseKind::Press
    }
}

/// The mode string that turns mouse reporting on, and the one that turns it
/// off again. `1000` reports presses and releases, `1002` adds drags (needed
/// for the sidebar divider), `1006` is the SGR encoding above.
pub const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
/// Everything [`MOUSE_ON`] turned on, turned off — plus hover tracking
/// (`?1003`), which the workbench never asks for but a previous program in the
/// same terminal may have: a mode left on outlives the program that set it.
pub const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

/// Parse one SGR 1006 report. `None` for anything else — including an
/// unterminated fragment, which the terminal's input buffer will complete on a
/// later read.
pub fn parse(data: &str) -> Option<MouseEvent> {
    let rest = data.strip_prefix("\x1b[<")?;
    let last = rest.chars().last()?;
    if last != 'M' && last != 'm' {
        return None;
    }
    let body = &rest[..rest.len() - last.len_utf8()];
    let mut parts = body.split(';');
    let button: u16 = parts.next()?.parse().ok()?;
    let x: u16 = parts.next()?.parse().ok()?;
    let y: u16 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    // 1-based on the wire; the frame is 0-based.
    let x = x.checked_sub(1)?;
    let y = y.checked_sub(1)?;

    let wheel = button & 64 != 0;
    let motion = button & 32 != 0;
    let button_bits = button & 3;
    let released = last == 'm' || (!wheel && button_bits == 3 && !motion);
    let kind = if wheel {
        match button_bits {
            0 => MouseKind::WheelUp,
            1 => MouseKind::WheelDown,
            // Horizontal wheel: not used yet, and not a click either.
            _ => return None,
        }
    } else if motion {
        if button_bits == 3 {
            MouseKind::Move
        } else {
            MouseKind::Drag
        }
    } else if released {
        MouseKind::Release
    } else {
        MouseKind::Press
    };

    Some(MouseEvent {
        kind,
        x,
        y,
        shift: button & 4 != 0,
        alt: button & 8 != 0,
        ctrl: button & 16 != 0,
    })
}

/// True when `data` starts a mouse report but has not arrived whole yet.
pub fn is_partial(data: &str) -> bool {
    data.starts_with("\x1b[<") && !data.ends_with('M') && !data.ends_with('m')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_left_press_decodes_to_zero_based_cells() {
        let event = parse("\x1b[<0;12;7M").expect("a press");
        assert_eq!(event.kind, MouseKind::Press);
        assert_eq!((event.x, event.y), (11, 6));
        assert!(!event.ctrl && !event.alt && !event.shift);
    }

    #[test]
    fn a_release_arrives_both_ways_terminals_encode_it() {
        // SGR's own release: the `m` final byte.
        let sgr = parse("\x1b[<0;3;4m").expect("release");
        assert_eq!(sgr.kind, MouseKind::Release);
        // X10's legacy form: button 3 in a press (`M`), which some terminals
        // still send even when 1006 was negotiated.
        let legacy = parse("\x1b[<3;3;4M").expect("release");
        assert_eq!(legacy.kind, MouseKind::Release);
    }

    #[test]
    fn the_wheel_is_its_own_kind() {
        let up = parse("\x1b[<64;5;5M").expect("wheel up");
        assert_eq!(up.kind, MouseKind::WheelUp);
        let down = parse("\x1b[<65;5;5M").expect("wheel down");
        assert_eq!(down.kind, MouseKind::WheelDown);
        // A horizontal wheel is reported but is not a click.
        assert!(parse("\x1b[<66;5;5M").is_none());
    }

    #[test]
    fn dragging_and_bare_motion_are_told_apart() {
        let drag = parse("\x1b[<32;40;10M").expect("drag");
        assert_eq!(drag.kind, MouseKind::Drag);
        let hover = parse("\x1b[<35;40;10M").expect("move");
        assert_eq!(hover.kind, MouseKind::Move);
    }

    #[test]
    fn modifiers_ride_along() {
        let event = parse("\x1b[<20;1;1M").expect("ctrl+shift press");
        assert!(event.ctrl && event.shift && !event.alt);
    }

    #[test]
    fn anything_that_is_not_a_complete_report_is_none() {
        for data in [
            "",
            "\x1b[",
            "\x1b[<",
            "\x1b[<0;1;1",
            "\x1b[<0;1;1X",
            "\x1b[<0;1M",
            "\x1b[<0;1;1;1M",
            "\x1b[<a;1;1M",
            // Row/column 0 cannot be addressed (the wire is 1-based).
            "\x1b[<0;0;1M",
        ] {
            assert!(parse(data).is_none(), "{data:?} must not decode");
        }
        assert!(is_partial("\x1b[<0;1;1"));
        assert!(!is_partial("\x1b[<0;1;1M"));
    }
}
