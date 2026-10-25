//! The workbench's layout arithmetic and its hit regions.
//!
//! Two jobs, both pure:
//!
//! * [`layout`] turns `(width, height)` into the rectangles of the seven bands
//!   the contract fixes (header, hairline, body, input, activity, status, hint)
//!   plus the optional sidebar and its divider — with the two documented
//!   breakpoints: narrower than [`NARROW_WIDTH`] drops the sidebar, shorter
//!   than [`SMALL_HEIGHT`] squeezes the header and the input box.
//! * [`HitMap`] records what every rectangle *is* while the frame is drawn, so
//!   a mouse event can be resolved back to an id without re-deriving geometry.
//!   Mouse support has to agree with what is on screen; a second, hand-written
//!   coordinate table is exactly the thing that drifts.

/// A rectangle in terminal cells, 0-based, origin top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn new(x: u16, y: u16, w: u16, h: u16) -> Self {
        Self { x, y, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// One past the last column.
    pub fn right(&self) -> u16 {
        self.x.saturating_add(self.w)
    }

    /// One past the last row.
    pub fn bottom(&self) -> u16 {
        self.y.saturating_add(self.h)
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        !self.is_empty() && x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The same rect narrowed to `[from, to)` columns, never wider than `self`.
    pub fn inset_x(&self, from: u16, to: u16) -> Rect {
        let x = self.x.saturating_add(from);
        let limit = self.right().saturating_sub(to);
        Rect::new(x, self.y, limit.saturating_sub(x), self.h)
    }

    /// The same rect narrowed by `left`/`right` columns and `top`/`bottom` rows.
    pub fn inset(&self, left: u16, right: u16, top: u16, bottom: u16) -> Rect {
        let inner = self.inset_x(left, right);
        let y = inner.y.saturating_add(top);
        let limit = inner.bottom().saturating_sub(bottom);
        Rect::new(inner.x, y, inner.w, limit.saturating_sub(y))
    }
}

/// Below this width the sidebar is dropped (and `ctrl+b` opens a full-screen
/// panel instead).
pub const NARROW_WIDTH: u16 = 88;
/// Below this height the header loses its wordmark block and the input box
/// loses its bottom rule.
pub const SMALL_HEIGHT: u16 = 24;
/// Sidebar width when nothing has been dragged.
pub const SIDEBAR_DEFAULT: u16 = 36;
/// The divider drag's range.
pub const SIDEBAR_MIN: u16 = 30;
pub const SIDEBAR_MAX: u16 = 48;
/// Columns of padding inside the chat area, left and right.
pub const CHAT_PADDING: u16 = 2;

/// The seven bands of a workbench frame, in rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Short-terminal mode: one-row header, no activity row, two-row input.
    pub compact: bool,
    pub header: Rect,
    /// The conversation (or the launchpad), not including the sidebar.
    pub body: Rect,
    pub divider: Option<Rect>,
    pub sidebar: Option<Rect>,
    /// The input box block (border + prompt + optional closing rule).
    pub input: Rect,
    /// The row the prompt text is drawn on.
    pub prompt_row: u16,
    pub activity: Option<Rect>,
    pub status: Rect,
    /// The hint row; absent in compact mode, where the status row carries it.
    pub hint: Option<Rect>,
    /// Width the sidebar was given (0 when it is hidden).
    pub sidebar_w: u16,
}

/// Lay out a `w` × `h` terminal. Everything is clamped: a 1×1 terminal gets
/// empty rectangles rather than an underflow.
pub fn layout(w: u16, h: u16, sidebar_w: u16) -> Layout {
    let compact = h < SMALL_HEIGHT;
    let sidebar_w = if w >= NARROW_WIDTH {
        sidebar_w.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
    } else {
        0
    };
    // A sidebar that would leave the conversation under 20 columns is not worth
    // its width: fall back to the full-width body.
    let sidebar_w = if sidebar_w > 0 && w.saturating_sub(sidebar_w + 1) < 20 {
        0
    } else {
        sidebar_w
    };

    let chat_w = w.saturating_sub(sidebar_w + if sidebar_w > 0 { 1 } else { 0 });
    let (divider, sidebar, chat_w) = if sidebar_w > 0 {
        (
            Some(Rect::new(chat_w, 0, 1, h)),
            Some(Rect::new(chat_w + 1, 0, sidebar_w, h)),
            chat_w,
        )
    } else {
        (None, None, w)
    };

    if compact {
        let header = Rect::new(0, 0, w, 1);
        let input_y = h.saturating_sub(4);
        let body_h = input_y.saturating_sub(1);
        let body = Rect::new(0, 1, chat_w, body_h);
        let input = Rect::new(0, input_y, chat_w, 2.min(h.saturating_sub(input_y)));
        return Layout {
            compact,
            header,
            body,
            divider: divider.map(|_| Rect::new(chat_w, 1, 1, body_h)),
            sidebar: sidebar.map(|rect| Rect::new(rect.x, 1, rect.w, body_h)),
            input,
            prompt_row: input_y.saturating_add(1),
            activity: None,
            status: Rect::new(0, h.saturating_sub(2), w, 1),
            hint: Some(Rect::new(0, h.saturating_sub(1), w, 1)),
            sidebar_w,
        };
    }

    // Tall layout: header 5 rows (its last row is the hairline), a blank row,
    // the body, then the input block, activity, status and hint.
    let header = Rect::new(0, 0, w, 5);
    let body_y = 6u16;
    let body_h = h.saturating_sub(13);
    let input_y = h.saturating_sub(7);
    Layout {
        compact,
        header,
        body: Rect::new(0, body_y, chat_w, body_h),
        divider: divider.map(|_| Rect::new(chat_w, body_y, 1, body_h)),
        sidebar: sidebar.map(|rect| Rect::new(rect.x, body_y, rect.w, body_h)),
        input: Rect::new(0, input_y, chat_w, 4.min(h.saturating_sub(input_y))),
        prompt_row: input_y.saturating_add(1),
        activity: Some(Rect::new(0, h.saturating_sub(3), w, 1)),
        status: Rect::new(0, h.saturating_sub(2), w, 1),
        hint: Some(Rect::new(0, h.saturating_sub(1), w, 1)),
        sidebar_w,
    }
}

/// What was drawn where, for mouse resolution.
///
/// Regions are recorded in paint order; [`HitMap::hit`] walks them backwards so
/// an overlay painted over the body wins over the body, exactly like the frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HitMap {
    regions: Vec<(Rect, String)>,
}

impl HitMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// Register `rect` under `id`. An empty rect is ignored: a zero-height row
    /// is not clickable, and recording it would only make `hit` slower.
    pub fn push(&mut self, rect: Rect, id: impl Into<String>) {
        if !rect.is_empty() {
            self.regions.push((rect, id.into()));
        }
    }

    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// The id painted last at `(x, y)`.
    pub fn hit(&self, x: u16, y: u16) -> Option<&str> {
        self.regions
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(x, y))
            .map(|(_, id)| id.as_str())
    }

    /// Every region, for tests and diagnostics.
    pub fn regions(&self) -> &[(Rect, String)] {
        &self.regions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_breakpoints_move_what_the_contract_says() {
        let tall = layout(120, 38, SIDEBAR_DEFAULT);
        assert!(!tall.compact);
        assert_eq!(tall.sidebar.map(|rect| rect.w), Some(SIDEBAR_DEFAULT));
        assert!(tall.activity.is_some());
        assert_eq!(tall.header.h, 5);
        assert_eq!(tall.input.h, 4);

        // Narrow: the sidebar goes, the chat takes the whole width.
        let narrow = layout(80, 38, SIDEBAR_DEFAULT);
        assert_eq!(narrow.sidebar_w, 0);
        assert!(narrow.sidebar.is_none());
        assert!(narrow.divider.is_none());
        assert_eq!(narrow.body.w, 80);

        // Short: one-row header, two-row input, no activity.
        let short = layout(120, 20, SIDEBAR_DEFAULT);
        assert!(short.compact);
        assert_eq!(short.header.h, 1);
        assert_eq!(short.input.h, 2);
        assert!(short.activity.is_none());
    }

    #[test]
    fn the_bands_tile_the_frame_without_overlap_or_holes() {
        for (w, h) in [(120u16, 38u16), (90, 24), (88, 23), (80, 20), (200, 60)] {
            let l = layout(w, h, SIDEBAR_DEFAULT);
            let mut rows: Vec<u16> = vec![0; h as usize];
            let mut paint = |rect: Rect| {
                for y in rect.y..rect.bottom() {
                    rows[y as usize] += 1;
                }
            };
            if l.compact {
                paint(l.header);
                paint(l.body);
                paint(l.input);
                paint(l.status);
                if let Some(hint) = l.hint {
                    paint(hint);
                }
            } else {
                paint(l.header);
                paint(l.body);
                paint(l.input);
                if let Some(activity) = l.activity {
                    paint(activity);
                }
                paint(l.status);
                if let Some(hint) = l.hint {
                    paint(hint);
                }
            }
            assert!(
                rows.iter().all(|count| *count <= 1),
                "{w}x{h}: overlapping bands: {rows:?}"
            );
            // The gap between the header and the body in the tall layout is the
            // one deliberate blank row; everything else is covered.
            let expected_blank = if l.compact { 0 } else { 1 };
            let blank = rows.iter().filter(|count| **count == 0).count();
            assert!(
                blank <= expected_blank,
                "{w}x{h}: {blank} unpainted rows: {rows:?}"
            );
        }
    }

    #[test]
    fn the_sidebar_sits_to_the_right_of_the_conversation() {
        let l = layout(120, 38, 36);
        let sidebar = l.sidebar.expect("a tall, wide frame keeps the sidebar");
        let divider = l.divider.expect("and its divider");
        assert_eq!(sidebar.right(), 120);
        assert_eq!(divider.right(), sidebar.x);
        assert_eq!(l.body.right(), divider.x);
        assert_eq!(l.body.w + 1 + sidebar.w, 120);
    }

    #[test]
    fn the_sidebar_range_is_clamped_and_never_eats_the_conversation() {
        assert_eq!(layout(120, 38, 10).sidebar_w, SIDEBAR_MIN);
        assert_eq!(layout(200, 38, 99).sidebar_w, SIDEBAR_MAX);
        // 88 wide with a 48-column sidebar leaves 39 for the chat: fine.
        assert_eq!(layout(88, 38, 48).sidebar_w, 48);
        // A width outside the range is clamped into it, not honoured verbatim.
        assert_eq!(layout(88, 38, 70).sidebar_w, SIDEBAR_MAX);
    }

    #[test]
    fn a_tiny_terminal_yields_empty_rectangles_rather_than_underflow() {
        for (w, h) in [(1u16, 1u16), (2, 2), (10, 3), (4, 5)] {
            let l = layout(w, h, SIDEBAR_DEFAULT);
            assert_eq!(l.sidebar_w, 0);
            assert!(l.body.bottom() <= h, "{w}x{h}: body escapes the frame");
            assert!(l.input.bottom() <= h, "{w}x{h}: input escapes the frame");
            assert!(l.status.y < h);
        }
    }

    #[test]
    fn a_hit_is_resolved_against_what_was_painted_last() {
        let mut map = HitMap::new();
        map.push(Rect::new(0, 0, 10, 5), "body");
        map.push(Rect::new(2, 2, 4, 2), "row.0");
        assert_eq!(map.hit(3, 3), Some("row.0"));
        assert_eq!(map.hit(8, 3), Some("body"));
        assert_eq!(map.hit(20, 3), None);
        // An empty rectangle is not a target.
        map.push(Rect::new(0, 9, 10, 0), "never");
        assert_eq!(map.hit(5, 9), None);
    }

    #[test]
    fn nesting_respects_the_outer_rect() {
        let rect = Rect::new(5, 2, 10, 4);
        let inner = rect.inset(2, 3, 1, 1);
        assert_eq!(inner, Rect::new(7, 3, 5, 2));
        // An inset larger than the rect collapses instead of wrapping.
        assert!(rect.inset(20, 20, 20, 20).is_empty());
    }
}
