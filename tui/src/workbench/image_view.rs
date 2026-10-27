//! Picture preview: an image as text cells, and the arithmetic behind zooming.
//!
//! A terminal has no pixels, so a picture is drawn as **half blocks**: each cell
//! is one `▀` whose foreground is the top pixel of the pair and whose background
//! is the bottom one. Two pixels per cell is the most a text surface can show,
//! and it needs nothing from the terminal beyond colour — no kitty graphics, no
//! iTerm2 images, no sixel. That is the point: the TUI's job here is a
//! *preview* that works in the terminal the reader already has, and the
//! alternate-screen frame it paints cannot be handed to a protocol that places
//! its own bitmap out of band.
//!
//! Colour is 24-bit when the terminal reports it and the 6×6×6 cube otherwise,
//! so the same code is right on `xterm-256color` as on a truecolor terminal.
//!
//! Zoom is a multiplier on the fitted size, and panning is the window into what
//! no longer fits. [`render`] returns exactly `box_w` columns and `box_h` rows —
//! the same invariant every other band keeps — so the caller can splice it into
//! the frame without a special case.

use crate::workbench::ansi::fit;
use crate::workbench::theme::Palette;

/// The largest cell grid [`render`] will draw, in cells. A zoom that would ask
/// for more than this is clamped: the picture is a preview, and a reader who
/// wants the pixels closes the TUI.
const MAX_CELLS_W: usize = 400;
const MAX_CELLS_H: usize = 200;

/// An image already decoded and scaled down by the caller.
///
/// `rgba` is row-major, four bytes per pixel — what `image::DynamicImage`
/// converts to in one call. The workbench never decodes anything itself: the
/// adapter does that off the event loop and hands over the pixels.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct ImagePixels {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for ImagePixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImagePixels")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

impl ImagePixels {
    /// Is there anything to draw?
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0 || self.rgba.len() < self.width * self.height * 4
    }

    /// The colour at a pixel, clamped to the image: a sampling mistake at the
    /// edge must not read past the buffer.
    fn at(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        let offset = (y * self.width + x) * 4;
        match self.rgba.get(offset..offset + 4) {
            Some(px) => (px[0], px[1], px[2]),
            None => (0, 0, 0),
        }
    }
}

/// The cell size an image occupies at `zoom`, fitted into `box_w` × `box_h`.
///
/// A cell is two pixels tall, so the aspect ratio is halved on the vertical:
/// terminal cells are about twice as tall as they are wide, which is what makes
/// a `▀` look square.
pub fn fitted_cells(
    width: usize,
    height: usize,
    box_w: usize,
    box_h: usize,
    zoom: u32,
) -> (usize, usize) {
    if width == 0 || height == 0 || box_w == 0 || box_h == 0 {
        return (0, 0);
    }
    let zoom = zoom.max(1) as f64;
    let scale_w = box_w as f64 / width as f64;
    let scale_h = (box_h as f64 * 2.0) / height as f64;
    let scale = scale_w.min(scale_h) * zoom;
    let cells_w = ((width as f64 * scale).round() as usize).max(1);
    let cells_h = (((height as f64 * scale) / 2.0).round() as usize).max(1);
    (cells_w.clamp(1, MAX_CELLS_W), cells_h.clamp(1, MAX_CELLS_H))
}

/// The window a zoomed image occupies: how far it can be panned in each axis.
pub fn pan_limit(cells_w: usize, cells_h: usize, box_w: usize, box_h: usize) -> (usize, usize) {
    (cells_w.saturating_sub(box_w), cells_h.saturating_sub(box_h))
}

/// Clamp a pan request to what the current zoom allows.
pub fn clamp_pan(
    pan: (i32, i32),
    cells_w: usize,
    cells_h: usize,
    box_w: usize,
    box_h: usize,
) -> (usize, usize) {
    let (limit_x, limit_y) = pan_limit(cells_w, cells_h, box_w, box_h);
    (
        pan.0.clamp(0, limit_x as i32) as usize,
        pan.1.clamp(0, limit_y as i32) as usize,
    )
}

/// Draw `pixels` into exactly `box_w` columns × `box_h` rows.
///
/// `zoom` scales the fitted size, `pan` (in cells) picks the window when the
/// picture is bigger than the box, and `true_color` chooses 24-bit or the
/// 6×6×6 cube. Rows are ANSI strings already padded to `box_w`, so they go
/// straight into the frame.
pub fn render(
    pixels: &ImagePixels,
    palette: &Palette,
    box_w: usize,
    box_h: usize,
    zoom: u32,
    pan: (i32, i32),
    true_color: bool,
) -> Vec<String> {
    if pixels.is_empty() || box_w == 0 || box_h == 0 {
        return vec![fit("", box_w); box_h];
    }
    let (cells_w, cells_h) = fitted_cells(pixels.width, pixels.height, box_w, box_h, zoom);
    let (pan_x, pan_y) = clamp_pan(pan, cells_w, cells_h, box_w, box_h);
    // A picture narrower than the box is centred; one that fills or overflows
    // the box starts at its own left edge.
    let visible_w = cells_w.min(box_w);
    let pad_left = (box_w - visible_w) / 2;
    let pad_right = box_w - visible_w - pad_left;

    let mut rows: Vec<String> = Vec::with_capacity(box_h);
    for row in 0..box_h {
        let cell_y = pan_y + row;
        let mut line = String::new();
        if cell_y >= cells_h {
            rows.push(fit("", box_w));
            continue;
        }
        if pad_left > 0 {
            line.push_str(&" ".repeat(pad_left));
        }
        // Two source pixels per cell, sampled by nearest neighbour.
        let top = (cell_y * 2 * pixels.height) / (cells_h * 2);
        let bottom = ((cell_y * 2 + 1) * pixels.height) / (cells_h * 2);
        let mut last: Option<(String, String)> = None;
        for column in 0..visible_w {
            let cell_x = pan_x + column;
            let x = (cell_x * pixels.width) / cells_w;
            let (tr, tg, tb) = pixels.at(x, top);
            let (br, bg, bb) = pixels.at(x, bottom);
            let fg = color(tr, tg, tb, true_color);
            let bg = color(br, bg, bb, true_color);
            if last.as_ref() != Some(&(fg.clone(), bg.clone())) {
                line.push_str(&format!("\x1b[{fg};{bg}m"));
                last = Some((fg, bg));
            }
            line.push('▀');
        }
        if last.is_some() {
            line.push_str(reset());
        }
        if pad_right > 0 {
            line.push_str(&" ".repeat(pad_right));
        }
        rows.push(fit(&line, box_w));
    }
    let _ = palette;
    rows
}

fn reset() -> &'static str {
    "\x1b[0m"
}
/// One colour as an SGR fragment: `38;2;r;g;b` (truecolor) or `38;5;n` (cube).
fn color(r: u8, g: u8, b: u8, true_color: bool) -> String {
    if true_color {
        format!("38;2;{r};{g};{b}")
    } else {
        format!("38;5;{}", cube_index(r, g, b))
    }
}

/// The 6×6×6 colour cube index for an RGB triple (the 216 colours every
/// 256-colour terminal has; the greyscale ramp and the base 16 are ignored —
/// a photograph needs the cube, not the named colours).
fn cube_index(r: u8, g: u8, b: u8) -> u8 {
    let step = |value: u8| -> u8 {
        // 0, 95, 135, 175, 215, 255 — the cube's own levels, not an even split.
        match value {
            0..=47 => 0,
            48..=114 => 1,
            115..=154 => 2,
            155..=194 => 3,
            195..=234 => 4,
            _ => 5,
        }
    };
    16 + 36 * step(r) + 6 * step(g) + step(b)
}

/// A one-line description of an image for the box's title (`1280×720 · 24-bit`).
pub fn describe(pixels: &ImagePixels) -> String {
    format!("{}×{}", pixels.width, pixels.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::theme::FUTURE_DARK;

    fn solid(width: usize, height: usize, (r, g, b): (u8, u8, u8)) -> ImagePixels {
        let mut rgba = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
        ImagePixels {
            width,
            height,
            rgba,
        }
    }

    const P: Palette = FUTURE_DARK;

    #[test]
    fn every_row_is_exactly_the_box_width_and_height() {
        let image = solid(64, 48, (200, 40, 90));
        for (w, h) in [(40usize, 12usize), (80, 24), (1, 1), (7, 3)] {
            for zoom in [1u32, 2, 4] {
                let rows = render(&image, &P, w, h, zoom, (0, 0), true);
                assert_eq!(rows.len(), h, "{w}x{h} z{zoom}");
                for row in &rows {
                    assert_eq!(
                        crate::workbench::ansi::width(row),
                        w,
                        "{w}x{h} z{zoom}: {row:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_wider_image_keeps_its_aspect_ratio() {
        // 4:1 (very wide) in a 40x20 box: it is width-limited, so it takes all
        // four... no — the ratio is width/height, and a cell is two pixels tall.
        let (w, h) = fitted_cells(400, 100, 40, 20, 1);
        assert_eq!(w, 40, "fills the width");
        assert_eq!(h, 5, "100/(400/40)/2 = 5 rows");
        // A tall image in the same box is height-limited.
        let (w, h) = fitted_cells(100, 400, 40, 20, 1);
        assert_eq!(h, 20, "fills the height");
        assert!(w < 40, "and gives up width: {w}");
    }

    #[test]
    fn zoom_multiplies_the_fit_and_never_goes_below_one() {
        let (base_w, base_h) = fitted_cells(100, 100, 20, 10, 1);
        let (z2_w, z2_h) = fitted_cells(100, 100, 20, 10, 2);
        assert!(
            z2_w >= base_w * 2 - 2 && z2_h >= base_h * 2 - 1,
            "{base_w}x{base_h} → {z2_w}x{z2_h}"
        );
        let (z0_w, z0_h) = fitted_cells(100, 100, 20, 10, 0);
        assert_eq!((z0_w, z0_h), (base_w, base_h), "zoom 0 is zoom 1");
        // A degenerate image has no cells at all rather than a panic.
        assert_eq!(fitted_cells(0, 0, 20, 10, 1), (0, 0));
        assert_eq!(fitted_cells(10, 10, 0, 0, 1), (0, 0));
    }

    #[test]
    fn a_panned_view_stays_inside_the_picture() {
        let image = solid(200, 200, (10, 120, 200));
        let (cells_w, cells_h) = fitted_cells(image.width, image.height, 40, 20, 4);
        let (limit_x, limit_y) = pan_limit(cells_w, cells_h, 40, 20);
        assert!(limit_x > 0 && limit_y > 0, "zoom 4 overflows a 40x20 box");
        // Asking past the edge is clamped, not wrapped: a pan is a window, and a
        // window that showed the far side twice would read as a glitch.
        assert_eq!(
            clamp_pan((9_999, 9_999), cells_w, cells_h, 40, 20),
            (limit_x, limit_y)
        );
        assert_eq!(clamp_pan((-5, -5), cells_w, cells_h, 40, 20), (0, 0));
    }

    #[test]
    fn a_small_image_is_centred_and_a_big_one_starts_at_its_edge() {
        let small = solid(4, 2, (255, 0, 0));
        let rows = render(&small, &P, 20, 4, 1, (0, 0), true);
        let first = crate::utils::strip_ansi_codes(&rows[0]);
        let text = first.trim_end_matches(' ');
        let leading = first.len() - first.trim_start_matches(' ').len();
        assert!(leading > 0, "a 4-cell picture is centred in 20: {first:?}");
        assert!(text.contains('▀'), "{first:?}");
    }

    #[test]
    fn colours_are_truecolor_or_the_6x6x6_cube() {
        let image = solid(2, 2, (255, 0, 0));
        let rows = render(&image, &P, 4, 2, 1, (0, 0), true);
        assert!(rows[0].contains("38;2;255;0;0"), "{:?}", rows[0]);
        let rows = render(&image, &P, 4, 2, 1, (0, 0), false);
        assert!(
            rows[0].contains("38;5;196"),
            "red in the cube: {:?}",
            rows[0]
        );
        assert!(!rows[0].contains("38;2;"));
        // The cube's own levels, not an even split.
        assert_eq!(cube_index(0, 0, 0), 16);
        assert_eq!(cube_index(255, 255, 255), 231);
        assert_eq!(cube_index(255, 0, 0), 196);
        assert_eq!(cube_index(95, 135, 175), 16 + 36 + 6 * 2 + 3);
    }

    #[test]
    fn an_empty_image_paints_the_box_and_nothing_else() {
        let empty = ImagePixels::default();
        let rows = render(&empty, &P, 10, 3, 1, (0, 0), true);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.trim().is_empty()));
        assert_eq!(describe(&empty), "0×0");
    }
}
