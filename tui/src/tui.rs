//! Core types the renderers share: ANSI escape constants, cursor/colour helpers
//! and the [`Component`] trait.
//!
//! What is left is exactly what the workbench draws with. The overlay layout
//! maths (`OverlayLayout`, anchors, margins), the focus plumbing and the theme
//! fan-out that walked an overlay stack all belonged to the inline UI's
//! compositor, which no longer exists: the workbench composes rows itself and
//! paints chrome from its own palette (`workbench::theme`).

// ─── ANSI Escape Sequences ─────────────────────────────────────────────────

pub const ESC: &str = "\x1b";
pub const CSI: &str = "\x1b[";
pub const CLEAR: &str = "\x1b[2J";
pub const CLEAR_LINE: &str = "\x1b[2K";
pub const CURSOR_HIDE: &str = "\x1b[?25l";
pub const CURSOR_SHOW: &str = "\x1b[?25h";
pub const RESET: &str = "\x1b[m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";

// Synchronized output — the terminal buffers everything written between
// begin/end and flushes it atomically, which is what keeps a full repaint from
// tearing.
pub const SYNC_BEGIN: &str = "\x1b[?2026h";
pub const SYNC_END: &str = "\x1b[?2026l";

pub fn cursor_pos(row: usize, col: usize) -> String {
    format!("{CSI}{row};{col}H")
}

pub fn set_fg(c: u8) -> String {
    format!("{CSI}38;5;{c}m")
}

pub fn set_bg(c: u8) -> String {
    format!("{CSI}48;5;{c}m")
}

// ─── Component ─────────────────────────────────────────────────────────────

/// ANSI "pi:c" cursor marker injected into rendered input lines; the renderer
/// strips it before writing to the terminal (see `workbench::run`).
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

/// A renderable element. `render` returns display rows (ANSI-styled strings);
/// `handle_input` receives normalised key ids (see `keys.rs`) or paste chunks.
///
/// The workbench treats components as *rectangles*: it asks one for rows of a
/// given width and places them, rather than letting a component decide how much
/// of the screen it takes.
pub trait Component: std::any::Any {
    fn render(&mut self, width: usize) -> Vec<String>;
    fn handle_input(&mut self, _data: &str) {}
    fn invalidate(&mut self) {}

    /// True when this component wants `escape` delivered to it before anything
    /// else acts on it (an active incremental search owns the first escape).
    fn wants_escape(&self) -> bool {
        false
    }

    /// True when this component wants key-release events.
    fn wants_key_release(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// Components that can hold keyboard focus (`{ focused: bool }`).
pub trait Focusable: Component {
    fn focused(&self) -> bool;
    fn set_focused(&mut self, focused: bool);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_colour_and_cursor_helpers_emit_the_256_colour_forms() {
        assert_eq!(set_fg(80), "\x1b[38;5;80m");
        assert_eq!(set_bg(235), "\x1b[48;5;235m");
        assert_eq!(cursor_pos(3, 7), "\x1b[3;7H");
    }

    #[test]
    fn the_defaults_of_the_component_trait_are_inert() {
        struct Blank;
        impl Component for Blank {
            fn render(&mut self, width: usize) -> Vec<String> {
                vec![" ".repeat(width)]
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }
        let mut blank = Blank;
        assert_eq!(blank.render(3), vec!["   "]);
        // No input, no invalidation, no special keys: a component that only
        // draws needs to implement exactly one method.
        blank.handle_input("x");
        blank.invalidate();
        assert!(!blank.wants_key_release());
        assert!(!blank.wants_escape());
    }
}
