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

/// Whether a terminal `w` columns wide can hold the sidebar at all.
///
/// One predicate for the compositor, the key handler and the tests: a width
/// that "hides" the sidebar while the key handler still believes it can be
/// shown is how `ctrl+b` ended up opening a full-screen overlay on a 120-column
/// terminal once the sidebar had been toggled off.
pub fn sidebar_fits(w: u16) -> bool {
    w >= NARROW_WIDTH
}
/// Below this height the header loses its wordmark block and the input box
/// loses its bottom rule.
pub const SMALL_HEIGHT: u16 = 24;
/// Sidebar width when nothing has been dragged.
pub const SIDEBAR_DEFAULT: u16 = 36;
/// The divider drag's range.
///
/// The ceiling is generous because the rail is where a file or a session row is
/// read, and 48 columns clipped what the reader dragged open to see. It still
/// leaves the conversation more than its 20-column floor at the narrowest frame
/// that shows a rail at all ([`NARROW_WIDTH`]): 88 − 60 − 1 = 27.
pub const SIDEBAR_MIN: u16 = 30;
pub const SIDEBAR_MAX: u16 = 60;
/// Columns of padding inside the chat area, left and right.
pub const CHAT_PADDING: u16 = 2;

/// Columns of padding each side of the conversation band `columns` wide.
///
/// A band narrower than eight columns cannot afford [`CHAT_PADDING`] twice and
/// still hold text, so the padding shrinks with it rather than eating the whole
/// band.
pub fn chat_padding(columns: u16) -> usize {
    (CHAT_PADDING as usize).min(columns as usize / 4)
}

/// The text width inside a conversation band `columns` wide.
///
/// The single definition of "how wide is the conversation": the compositor pads
/// by [`chat_padding`] and the transcript is wrapped to exactly what is left.
/// Two places deriving it independently is how rows get wrapped at one width
/// and drawn at another — and a *probe* width (the driver loop used to ask for
/// a whole frame at width `1` just to read one field) is how the transcript
/// loses its per-width cache on every event.
pub fn chat_inner(columns: u16) -> usize {
    (columns as usize)
        .saturating_sub(chat_padding(columns) * 2)
        .max(1)
}

/// What painted what where: the geometry one frame is composed for.
///
/// Passed whole rather than as loose numbers because the two consumers must
/// agree: the transcript caches its rendered rows per *width*, so a caller that
/// hands it a width other than [`Self::inner`] throws those rows away and
/// re-wraps every message. The type is what makes that mistake unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub width: u16,
    pub height: u16,
    /// The conversation band's text width — see [`chat_inner`].
    pub inner: usize,
    /// The band rectangles this frame's `(width, height)` produce.
    pub layout: Layout,
}

impl Frame {
    /// The frame a `width` × `height` terminal gets with a sidebar of
    /// `sidebar_w` columns (0 for none), laid out with the reader's density and
    /// rail position. Both sizes are clamped up to 1, so a report of `0` from a
    /// terminal that has not answered `TIOCGWINSZ` yet cannot produce a
    /// zero-width frame.
    pub fn new(
        width: u16,
        height: u16,
        sidebar_w: u16,
        density: super::preferences::Density,
        position: super::preferences::SidebarPosition,
    ) -> Self {
        let width = width.max(1);
        let height = height.max(1);
        let layout = layout_with_preferences(width, height, sidebar_w, density, position);
        Self {
            width,
            height,
            inner: chat_inner(layout.body.w),
            layout,
        }
    }
}

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
    layout_with_preferences(
        w,
        h,
        sidebar_w,
        super::preferences::Density::Focused,
        super::preferences::SidebarPosition::Right,
    )
}

/// Lay out with the reader's density and rail position. The conversation always
/// gets the reclaimed space: density changes the chrome, never the text width.
pub fn layout_with_preferences(
    w: u16,
    h: u16,
    sidebar_w: u16,
    density: super::preferences::Density,
    position: super::preferences::SidebarPosition,
) -> Layout {
    let compact = h < SMALL_HEIGHT;
    // `0` is not a width to clamp — it is "no sidebar": `ctrl+b` closes the
    // rail by asking for none, and clamping that up to `SIDEBAR_MIN` is how a
    // closed sidebar kept 30 columns of screen while claiming to be hidden.
    let sidebar_w = if sidebar_w == 0 || !sidebar_fits(w) {
        0
    } else {
        sidebar_w.clamp(SIDEBAR_MIN, SIDEBAR_MAX)
    };
    // A sidebar that would leave the conversation under 20 columns is not worth
    // its width: fall back to the full-width body.
    let sidebar_w = if sidebar_w > 0 && w.saturating_sub(sidebar_w + 1) < 20 {
        0
    } else {
        sidebar_w
    };

    let chat_w = w.saturating_sub(sidebar_w + if sidebar_w > 0 { 1 } else { 0 });
    let left = position == super::preferences::SidebarPosition::Left;
    let chat_x = if left && sidebar_w > 0 {
        sidebar_w + 1
    } else {
        0
    };
    let (divider, sidebar, chat_w) = if sidebar_w > 0 {
        (
            Some(Rect::new(if left { sidebar_w } else { chat_w }, 0, 1, h)),
            Some(Rect::new(
                if left { 0 } else { chat_w + 1 },
                0,
                sidebar_w,
                h,
            )),
            chat_w,
        )
    } else {
        (None, None, w)
    };

    if compact {
        let header = Rect::new(0, 0, w, 1);
        let input_y = h.saturating_sub(4);
        let body_h = input_y.saturating_sub(1);
        let body = Rect::new(chat_x, 1, chat_w, body_h);
        let input = Rect::new(chat_x, input_y, chat_w, 2.min(h.saturating_sub(input_y)));
        // The rail (and its divider) run down to the footer: they cover the
        // composer band as well as the body, so the column beside the box is the
        // rail's own background rather than canvas. The three footer rows
        // (activity / status / hint) stay full width — they are a footer, and a
        // footer that starts a third of the way across reads as a mistake.
        let rail_h = h.saturating_sub(3).saturating_sub(1);
        return Layout {
            compact,
            header,
            body,
            divider: divider.map(|rect| Rect::new(rect.x, 1, 1, rail_h)),
            sidebar: sidebar.map(|rect| Rect::new(rect.x, 1, rect.w, rail_h)),
            input,
            prompt_row: input_y.saturating_add(1),
            activity: None,
            status: Rect::new(0, h.saturating_sub(2), w, 1),
            hint: Some(Rect::new(0, h.saturating_sub(1), w, 1)),
            sidebar_w,
        };
    }

    // Tall layout: header 6 rows (five identity rows, then the hairline), a blank
    // row, the body, then the input block, activity, status and hint.
    //
    // Six, not five: the header's identity block is the 5-row `FUTURE OS` block
    // font, so a 5-row header drew only the top four rows
    // of every letter — the bottom stroke of F, U, E and S was cut off, which
    // reads as clipped art rather than as a frame. The launchpad's separate
    // four-row hero does not change the chat header's five-row identity block.
    let header_h = if density == super::preferences::Density::Comfortable {
        6
    } else {
        2
    };
    let header = Rect::new(0, 0, w, header_h);
    let body_y = header_h + 1;
    // The composer band is exactly the box it draws: a top rule, the prompt
    // row, the closing rule. It used to reserve four rows for three, and the
    // spare one stayed canvas — a blank line between the box and the activity
    // row that read as a gap in the frame.
    let input_y = h.saturating_sub(6);
    let body_h = input_y.saturating_sub(body_y);
    // Down to the footer: the rail covers the composer band too (see the
    // compact branch).
    let rail_h = h.saturating_sub(3).saturating_sub(body_y);
    Layout {
        compact,
        header,
        body: Rect::new(chat_x, body_y, chat_w, body_h),
        divider: divider.map(|rect| Rect::new(rect.x, body_y, 1, rail_h)),
        sidebar: sidebar.map(|rect| Rect::new(rect.x, body_y, rect.w, rail_h)),
        input: Rect::new(chat_x, input_y, chat_w, 3.min(h.saturating_sub(input_y))),
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

    /// Where the next `push` will land.
    ///
    /// Hand it to [`Self::hit_after`] to resolve a pointer against only what
    /// was painted later — which is how a modal keeps its own surface separate
    /// from the page it covers.
    pub fn mark(&self) -> usize {
        self.regions.len()
    }

    /// The id painted last at `(x, y)` at or after `from`.
    ///
    /// `hit_after(x, y, mark())` is `hit(x, y)`; `hit_after(x, y, 0)` is the
    /// same answer over the whole frame. The one that matters is the mark taken
    /// before an overlay was painted: a region *under* the overlay must not
    /// answer a click that landed on the overlay.
    pub fn hit_after(&self, x: u16, y: u16, from: usize) -> Option<&str> {
        self.regions
            .get(from..)
            .unwrap_or_default()
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
    use crate::workbench::preferences::{Density, SidebarPosition};

    #[test]
    fn the_two_breakpoints_move_what_the_contract_says() {
        // One predicate for the compositor, the key handler and the tests.
        assert!(!sidebar_fits(87));
        assert!(sidebar_fits(88));
        let tall = layout(120, 38, SIDEBAR_DEFAULT);
        assert!(!tall.compact);
        assert_eq!(tall.sidebar.map(|rect| rect.w), Some(SIDEBAR_DEFAULT));
        assert!(tall.activity.is_some());
        assert_eq!(tall.header.h, 2);
        assert_eq!(tall.input.h, 3, "the box's own rows, and no spare");

        // Narrow: the sidebar goes, the chat takes the whole width.
        let narrow = layout(80, 38, SIDEBAR_DEFAULT);
        assert_eq!(narrow.sidebar_w, 0);
        assert!(narrow.sidebar.is_none());
        assert!(narrow.divider.is_none());
        assert_eq!(narrow.body.w, 80);

        // `0` asks for no rail at all on a terminal that could hold one.
        let closed = layout(120, 38, 0);
        assert_eq!(closed.sidebar_w, 0);
        assert!(closed.sidebar.is_none());
        assert_eq!(closed.body.w, 120);

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
        // (The ceiling itself is 60 — see `SIDEBAR_MAX` — and still leaves 27.)
        assert_eq!(layout(88, 38, 48).sidebar_w, 48);
        // A width outside the range is clamped into it, not honoured verbatim.
        assert_eq!(layout(88, 38, 70).sidebar_w, SIDEBAR_MAX);
    }

    #[test]
    fn preferences_reclaim_space_and_move_the_whole_rail() {
        let focused = layout(120, 38, SIDEBAR_DEFAULT);
        let comfortable = layout_with_preferences(
            120,
            38,
            SIDEBAR_DEFAULT,
            super::super::preferences::Density::Comfortable,
            super::super::preferences::SidebarPosition::Right,
        );
        assert_eq!(focused.header.h, 2);
        assert_eq!(focused.body.h, comfortable.body.h + 4);
        assert_eq!(focused.body.w, comfortable.body.w);
        let left = layout_with_preferences(
            120,
            38,
            SIDEBAR_DEFAULT,
            super::super::preferences::Density::Focused,
            super::super::preferences::SidebarPosition::Left,
        );
        assert_eq!(left.sidebar.unwrap().x, 0);
        assert_eq!(left.divider.unwrap().x, SIDEBAR_DEFAULT);
        assert_eq!(left.body.x, SIDEBAR_DEFAULT + 1);
        assert_eq!(left.input.x, left.body.x);
        assert_eq!(left.body.right(), 120);
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

    /// The conversation's text width is one definition, and a frame carries it:
    /// the compositor pads, the renderer wraps, and both read `chat_inner`.
    #[test]
    fn a_frame_carries_the_width_the_conversation_is_drawn_at() {
        for (w, h, sidebar) in [
            (120u16, 38u16, SIDEBAR_DEFAULT),
            (120, 38, 0),
            (90, 24, SIDEBAR_DEFAULT),
            (60, 20, 0),
            (24, 10, 0),
            (8, 4, 0),
            (7, 4, 0),
            (1, 1, 0),
            (0, 0, 0),
        ] {
            let frame = Frame::new(w, h, sidebar, Density::Focused, SidebarPosition::Right);
            assert_eq!(
                frame.inner,
                chat_inner(frame.layout.body.w),
                "the frame must name the width its own band is drawn at: {w}x{h}"
            );
            assert!(frame.inner >= 1, "never a zero-width conversation");
            assert_eq!(frame.width, w.max(1));
            assert_eq!(frame.height, h.max(1));
        }
    }

    /// The width the driver wraps the transcript at is the width the compositor
    /// has room for — for every ordinary terminal that is exactly "band minus
    /// the two padding columns", i.e. what the driver used to compute by hand.
    #[test]
    fn chat_inner_is_the_band_minus_its_padding() {
        for columns in [200u16, 120, 97, 88, 80, 40, 20, 8] {
            assert_eq!(
                chat_inner(columns),
                columns as usize - CHAT_PADDING as usize * 2,
                "{columns} columns"
            );
            assert_eq!(chat_padding(columns), CHAT_PADDING as usize);
        }
    }

    #[test]
    fn chat_inner_takes_no_more_padding_than_a_narrow_band_can_afford() {
        // Under 8 columns the padding shrinks rather than eating the band.
        assert_eq!(chat_inner(7), 5);
        assert_eq!(chat_padding(7), 1);
        assert_eq!(chat_inner(4), 2);
        assert_eq!(chat_inner(3), 3);
        // A degenerate band still yields a width the renderer can use.
        assert_eq!(chat_inner(1), 1);
        assert_eq!(chat_inner(0), 1);
    }
}
