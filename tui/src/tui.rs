//! Core TUI types and utilities — 1:1 port of `tui/src/tui.ts` (the parts
//! that don't touch the Node terminal): ANSI escape constants, cursor/color
//! helpers, the `Component` architecture, and overlay positioning math.
//!
//! The Node `Terminal` interface + `NodeTerminal` implementation live in
//! `terminal.rs` (self-implemented backend). The `Theme` table is in
//! `theme.rs`. `InputListener` types are app-layer concerns (P2).

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

// Synchronized output — terminal buffers all writes between begin/end
// and flushes atomically, preventing flicker and tearing.
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

// ─── Component Architecture ─────────────────────────────────────────────────

/// ANSI "pi:c" cursor marker injected into rendered input lines; the
/// renderer strips it before writing to the terminal (see `render`).
pub const CURSOR_MARKER: &str = "\x1b_pi:c\x07";

/// A renderable UI element. `render` returns display rows (ANSI-styled
/// strings); `handle_input` receives normalized key ids (see `keys.rs`) or
/// paste chunks; `invalidate` drops any cached layout.
pub trait Component: std::any::Any {
    fn render(&mut self, width: usize) -> Vec<String>;
    fn handle_input(&mut self, _data: &str) {}
    fn invalidate(&mut self) {}

    /// Port of the TS `wantsKeyRelease?: boolean` optional component field
    /// (default false). Only consulted by the app layer (P3).
    fn wants_key_release(&self) -> bool {
        false
    }

    /// Downcast support (mirrors TS duck-typing like `"focused" in component`).
    fn as_any(&self) -> &dyn std::any::Any;

    /// Mutable downcast support — used by the app layer to flip the `focused`
    /// flag on boxed overlay components (mirrors TS `component.focused = true`).
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// Set/clear the `focused` flag on a component, mirroring the TS
/// `isFocusable(component)` guard + `component.focused = ...` assignment.
/// Only the focusable components (Input, ScopedModelsSelector) are affected.
pub fn set_component_focused(component: &mut dyn Component, focused: bool) {
    if let Some(input) = component
        .as_any_mut()
        .downcast_mut::<crate::components::input::Input>()
    {
        input.focused = focused;
    } else if let Some(sel) = component
        .as_any_mut()
        .downcast_mut::<crate::components::scoped_models_selector::ScopedModelsSelector>(
    ) {
        sel.focused = focused;
    }
}

/// Components that can receive keyboard focus (port of the TS `Focusable`
/// interface — `{ focused: boolean }`).
pub trait Focusable: Component {
    fn focused(&self) -> bool;
    fn set_focused(&mut self, focused: bool);
}

/// Port of `isFocusable(component)` — TS checks `"focused" in component` at
/// runtime; in Rust we downcast against the known focusable components.
pub fn is_focusable(component: &dyn Component) -> bool {
    component.as_any().is::<crate::components::input::Input>()
        || component
            .as_any()
            .is::<crate::components::scoped_models_selector::ScopedModelsSelector>()
}

/// Simple container that renders children top-to-bottom (port of `Container`).
#[derive(Default)]
pub struct Container {
    pub children: Vec<Box<dyn Component>>,
}

impl Container {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }

    pub fn add_child(&mut self, component: Box<dyn Component>) {
        self.children.push(component);
    }

    /// TS `removeChild(component)` removes by object identity; in Rust we
    /// remove by index (the app layer keeps track of its own children).
    pub fn remove_child_at(&mut self, index: usize) -> Option<Box<dyn Component>> {
        if index < self.children.len() {
            Some(self.children.remove(index))
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        self.children.clear();
    }
}

impl Component for Container {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for child in &mut self.children {
            for line in child.render(width) {
                lines.push(line);
            }
        }
        lines
    }

    fn invalidate(&mut self) {
        for child in &mut self.children {
            child.invalidate();
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ─── Overlay Positioning ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAnchor {
    Center,
    TopLeft,
    TopRight,
    TopCenter,
    BottomLeft,
    BottomRight,
    BottomCenter,
    LeftCenter,
    RightCenter,
}

// TS `anchor ?? "center"` — the default anchor. Manual impl (clippy's
// derivable_impls suggestion needs nightly for non-unit variants).
#[allow(clippy::derivable_impls)]
impl Default for OverlayAnchor {
    fn default() -> Self {
        Self::Center
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OverlayMargin {
    pub top: Option<usize>,
    pub right: Option<usize>,
    pub bottom: Option<usize>,
    pub left: Option<usize>,
}

/// TS `OverlayMargin | number` — a plain number applies to all four sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarginValue {
    All(usize),
    Sides(OverlayMargin),
}

/// Port of `SizeValue` = `number | \`${number}%\``. Invalid strings parse to
/// 0 in TS; the enum only models the two valid forms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeValue {
    Fixed(usize),
    Percent(f64),
}

pub fn parse_size_value(v: &SizeValue, total: i64) -> i64 {
    match v {
        SizeValue::Fixed(n) => *n as i64,
        SizeValue::Percent(pct) => ((pct / 100.0) * total as f64).floor() as i64,
    }
}

#[derive(Debug, Clone, Default)]
pub struct OverlayOptions {
    pub width: Option<SizeValue>,
    pub min_width: Option<usize>,
    pub max_height: Option<SizeValue>,
    pub anchor: Option<OverlayAnchor>,
    pub offset_x: Option<i64>,
    pub offset_y: Option<i64>,
    pub row: Option<SizeValue>,
    pub col: Option<SizeValue>,
    pub margin: Option<MarginValue>,
    /// `visible(termWidth, termHeight)` gate — app-layer (P2); kept as a doc
    /// note since a bare fn pointer can't capture state.
    pub non_capturing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayLayout {
    pub row: usize,
    pub col: usize,
    pub width: usize,
    pub max_height: usize,
}

pub fn resolve_overlay_layout(
    term_width: usize,
    term_height: usize,
    component_lines: usize,
    options: Option<&OverlayOptions>,
) -> OverlayLayout {
    let opt = options.cloned().unwrap_or_default();

    let (m_top, m_right, m_bottom, m_left) = match &opt.margin {
        None => (0usize, 0usize, 0usize, 0usize),
        Some(MarginValue::All(n)) => (*n, *n, *n, *n),
        Some(MarginValue::Sides(m)) => (
            m.top.unwrap_or(0),
            m.right.unwrap_or(0),
            m.bottom.unwrap_or(0),
            m.left.unwrap_or(0),
        ),
    };

    // JS number math — widths can go negative; clamp at the end like TS.
    let avail_w = term_width as i64 - m_left as i64 - m_right as i64;
    let avail_h = term_height as i64 - m_top as i64 - m_bottom as i64;

    let mut w = match opt.width {
        Some(v) => parse_size_value(&v, term_width as i64),
        None => 80.min(avail_w),
    };
    if let Some(min_width) = opt.min_width {
        w = w.max(min_width as i64);
    }
    w = 1.max(w.min(avail_w));

    let mut max_h = match opt.max_height {
        Some(v) => parse_size_value(&v, term_height as i64),
        None => term_height as i64,
    };
    max_h = max_h.min(avail_h);

    let effective_h = (component_lines as i64).min(max_h);
    let anchor = opt.anchor.unwrap_or(OverlayAnchor::Center);

    let row = match opt.row {
        Some(v) => parse_size_value(&v, avail_h),
        None => match anchor {
            OverlayAnchor::TopLeft | OverlayAnchor::TopRight | OverlayAnchor::TopCenter => {
                m_top as i64
            }
            OverlayAnchor::BottomLeft
            | OverlayAnchor::BottomRight
            | OverlayAnchor::BottomCenter => m_top as i64 + avail_h - effective_h,
            _ => m_top as i64 + ((avail_h - effective_h) as f64 / 2.0).floor() as i64,
        },
    };

    let col = match opt.col {
        Some(v) => parse_size_value(&v, avail_w),
        None => match anchor {
            OverlayAnchor::TopLeft | OverlayAnchor::BottomLeft | OverlayAnchor::LeftCenter => {
                m_left as i64
            }
            OverlayAnchor::TopRight | OverlayAnchor::BottomRight | OverlayAnchor::RightCenter => {
                m_left as i64 + avail_w - w
            }
            _ => m_left as i64 + ((avail_w - w) as f64 / 2.0).floor() as i64,
        },
    };

    let mut row = row + opt.offset_y.unwrap_or(0);
    let mut col = col + opt.offset_x.unwrap_or(0);
    // TS: Math.max(0, Math.min(row, termHeight - 1)) — clamp inside first.
    row = row.min(term_height as i64 - 1).max(0);
    col = col.min(term_width as i64 - 1).max(0);

    OverlayLayout {
        row: row as usize,
        col: col as usize,
        width: w.max(0) as usize,
        max_height: max_h.max(0) as usize,
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_pos_formats_1_based() {
        assert_eq!(cursor_pos(3, 5), "\x1b[3;5H");
    }

    #[test]
    fn set_fg_bg_use_256_color() {
        assert_eq!(set_fg(151), "\x1b[38;5;151m");
        assert_eq!(set_bg(59), "\x1b[48;5;59m");
    }

    #[test]
    fn parse_size_value_fixed_and_percent() {
        assert_eq!(parse_size_value(&SizeValue::Fixed(10), 80), 10);
        assert_eq!(parse_size_value(&SizeValue::Percent(50.0), 80), 40);
        // floor: 33% of 100 = 33; 34% of 100 = 34
        assert_eq!(parse_size_value(&SizeValue::Percent(33.0), 100), 33);
        // Negative totals are allowed (JS number math).
        assert_eq!(parse_size_value(&SizeValue::Percent(50.0), -10), -5);
    }

    #[test]
    fn overlay_default_centers() {
        let layout = resolve_overlay_layout(100, 40, 10, None);
        // width = min(80, 100) = 80; row = floor((40-10)/2) = 15; col = floor((100-80)/2) = 10
        assert_eq!(
            layout,
            OverlayLayout {
                row: 15,
                col: 10,
                width: 80,
                max_height: 40
            }
        );
    }

    #[test]
    fn overlay_respects_margin_and_max_height() {
        let options = OverlayOptions {
            margin: Some(MarginValue::All(2)),
            max_height: Some(SizeValue::Percent(50.0)),
            ..Default::default()
        };
        let layout = resolve_overlay_layout(100, 40, 30, Some(&options));
        // avail 96x36; maxH = 20; effectiveH = min(30, 20) = 20
        // row = 2 + floor((36-20)/2) = 10; col = 2 + floor((96-80)/2) = 10
        assert_eq!(
            layout,
            OverlayLayout {
                row: 10,
                col: 10,
                width: 80,
                max_height: 20
            }
        );
    }

    #[test]
    fn overlay_top_left_anchor() {
        let options = OverlayOptions {
            anchor: Some(OverlayAnchor::TopLeft),
            width: Some(SizeValue::Fixed(50)),
            ..Default::default()
        };
        let layout = resolve_overlay_layout(100, 40, 10, Some(&options));
        assert_eq!(layout.row, 0);
        assert_eq!(layout.col, 0);
        assert_eq!(layout.width, 50);
    }

    #[test]
    fn overlay_bottom_right_anchor_and_offset() {
        let options = OverlayOptions {
            anchor: Some(OverlayAnchor::BottomRight),
            width: Some(SizeValue::Fixed(50)),
            offset_x: Some(-2),
            offset_y: Some(-1),
            ..Default::default()
        };
        let layout = resolve_overlay_layout(100, 40, 10, Some(&options));
        // col = 0 + 100 - 50 - 2 = 48; row = 0 + 40 - 10 - 1 = 29
        assert_eq!(
            layout,
            OverlayLayout {
                row: 29,
                col: 48,
                width: 50,
                max_height: 40
            }
        );
    }

    #[test]
    fn overlay_clamps_to_terminal_bounds() {
        // width overflows avail: w = max(1, min(80, -10)) = 1
        let options = OverlayOptions {
            width: Some(SizeValue::Fixed(200)),
            margin: Some(MarginValue::All(60)),
            ..Default::default()
        };
        let layout = resolve_overlay_layout(100, 40, 10, Some(&options));
        assert_eq!(layout.width, 1);
        assert!(layout.col <= 99);
    }

    #[test]
    fn container_renders_children_in_order() {
        struct FixedLine(String);
        impl Component for FixedLine {
            fn render(&mut self, _width: usize) -> Vec<String> {
                vec![self.0.clone()]
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }

        let mut c = Container::new();
        c.add_child(Box::new(FixedLine("a".into())));
        c.add_child(Box::new(FixedLine("b".into())));
        assert_eq!(c.render(80), vec!["a", "b"]);
        // as_any downcasts work on the container and its children.
        assert!(c.as_any().downcast_ref::<Container>().is_some());
        assert!(c.as_any_mut().downcast_mut::<Container>().is_some());
        // remove_child_at returns the component and bounds-checks the index.
        let mut removed = c.remove_child_at(0).unwrap();
        assert!(removed.as_any().downcast_ref::<FixedLine>().is_some());
        assert!(removed.as_any_mut().downcast_mut::<FixedLine>().is_some());
        assert!(c.remove_child_at(9).is_none());
        assert_eq!(c.render(80), vec!["b"]);
        c.clear();
        assert!(c.render(80).is_empty());
    }

    #[test]
    fn component_default_methods_are_noops() {
        struct Bare;
        impl Component for Bare {
            fn render(&mut self, _width: usize) -> Vec<String> {
                vec!["x".into()]
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }
        let mut c = Bare;
        c.handle_input("anything"); // default: ignored
        c.invalidate(); // default: no cache to drop
        assert!(!c.wants_key_release());
        assert_eq!(c.render(80), vec!["x"]);
        assert!(c.as_any().downcast_ref::<Bare>().is_some());
        assert!(c.as_any_mut().downcast_mut::<Bare>().is_some());
    }

    #[test]
    fn container_invalidate_propagates_to_children() {
        use std::cell::Cell;
        use std::rc::Rc;

        struct CountingChild(Rc<Cell<usize>>);
        impl Component for CountingChild {
            fn render(&mut self, _width: usize) -> Vec<String> {
                vec![]
            }
            fn invalidate(&mut self) {
                self.0.set(self.0.get() + 1);
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }

        let count = Rc::new(Cell::new(0));
        let mut c = Container::new();
        let mut child = CountingChild(Rc::clone(&count));
        assert!(child.as_any().downcast_ref::<CountingChild>().is_some());
        assert!(child.as_any_mut().downcast_mut::<CountingChild>().is_some());
        assert!(child.render(80).is_empty());
        c.add_child(Box::new(child));
        c.add_child(Box::new(CountingChild(Rc::clone(&count))));
        c.invalidate();
        assert_eq!(count.get(), 2);
    }

    #[test]
    fn focus_helpers_recognize_focusable_components() {
        use crate::components::input::Input;
        use crate::components::scoped_models_selector::{
            ScopedModelsSelector, ScopedModelsSelectorOptions,
        };
        use std::collections::HashSet;

        let mut input: Box<dyn Component> = Box::<Input>::default();
        assert!(is_focusable(input.as_ref()));
        set_component_focused(input.as_mut(), true);
        assert!(input.as_any().downcast_ref::<Input>().unwrap().focused);

        let mut selector: Box<dyn Component> =
            Box::new(ScopedModelsSelector::new(ScopedModelsSelectorOptions {
                all_models: vec![],
                enabled_model_ids: HashSet::new(),
                on_save: Box::new(|_| {}),
                on_cancel: Box::new(|| {}),
                max_visible: None,
            }));
        assert!(is_focusable(selector.as_ref()));
        set_component_focused(selector.as_mut(), true);
        assert!(
            selector
                .as_any()
                .downcast_ref::<ScopedModelsSelector>()
                .unwrap()
                .focused
        );

        struct Bare;
        impl Component for Bare {
            fn render(&mut self, _width: usize) -> Vec<String> {
                vec![]
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
            fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
                self
            }
        }
        let mut bare: Box<dyn Component> = Box::new(Bare);
        assert!(!is_focusable(bare.as_ref()));
        set_component_focused(bare.as_mut(), true); // no-op, must not panic
        assert!(bare.render(80).is_empty());
    }

    #[test]
    fn overlay_anchor_defaults_to_center() {
        assert_eq!(OverlayAnchor::default(), OverlayAnchor::Center);
    }

    #[test]
    fn overlay_layout_sides_margin_min_width_and_explicit_row_col() {
        let opts = OverlayOptions {
            margin: Some(MarginValue::Sides(OverlayMargin {
                top: Some(2),
                right: Some(4),
                bottom: None,
                left: Some(6),
            })),
            min_width: Some(30),
            row: Some(SizeValue::Fixed(5)),
            col: Some(SizeValue::Percent(50.0)),
            width: Some(SizeValue::Fixed(10)),
            ..Default::default()
        };
        let layout = resolve_overlay_layout(100, 40, 10, Some(&opts));
        // width 10 is bumped to min_width 30 (avail_w = 100-6-4 = 90).
        assert_eq!(layout.width, 30);
        assert_eq!(layout.row, 5);
        // col = 50% of avail_w 90.
        assert_eq!(layout.col, 45);
        // avail_h = 40-2-0 = 38 clamps the default max_height.
        assert_eq!(layout.max_height, 38);
    }
}
