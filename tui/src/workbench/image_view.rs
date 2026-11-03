//! Picture preview arithmetic: how big a picture is, where a zoomed window
//! starts, and what the box says about it.
//!
//! The picture itself is not drawn here, and deliberately so. A terminal has no
//! pixels of its own to paint; the only way to show a *picture* rather than a
//! approximation of one is the terminal's image protocol, which places a bitmap
//! at a cell out of band (see `crate::terminal_image::encode_kitty_rgb`, and
//! `workbench::screen::ImageRequest` for the placement the box asks for). This
//! module is the arithmetic that placement needs: the cell size an image fits
//! into, the window a zoomed one shows, and the numbers the box's title states.
//!
//! The half-block renderer that used to live here drew one `▀` per cell pair —
//! two pixels per cell, the most a text surface can show. It was removed: at a
//! hundred columns that is a 200-pixel-wide mosaic of a 2000-pixel photograph,
//! and a reader who opens a picture wants to see it, not to guess at it. A
//! terminal that cannot place a bitmap now says so instead.

/// The largest cell grid a fitted picture may occupy, in cells. A zoom that
/// would ask for more than this is clamped: the picture is a preview, and a
/// reader who wants every pixel closes the TUI.
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
    pub rgba: std::sync::Arc<[u8]>,
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
    /// Is there anything to show?
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0 || self.rgba.len() < self.width * self.height * 4
    }
}

/// The cell size an image occupies at `zoom`, fitted into `box_w` × `box_h`.
///
/// A cell is two pixels tall, so the aspect ratio is halved on the vertical:
/// terminal cells are about twice as tall as they are wide, which is what makes
/// a picture fitted this way look right on screen.
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

/// A one-line description of an image for the box's title (`1280×720`).
pub fn describe(pixels: &ImagePixels) -> String {
    format!("{}×{}", pixels.width, pixels.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: usize, height: usize, (r, g, b): (u8, u8, u8)) -> ImagePixels {
        let mut rgba = Vec::with_capacity(width * height * 4);
        for _ in 0..width * height {
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
        ImagePixels {
            width,
            height,
            rgba: rgba.into(),
        }
    }

    #[test]
    fn a_wider_image_keeps_its_aspect_ratio() {
        // 4:1 (very wide) in a 40x20 box: it is width-limited, so it takes all
        // forty columns, and a cell is two pixels tall.
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
    fn an_empty_image_has_no_cells_and_says_zero_by_zero() {
        let empty = ImagePixels::default();
        assert!(empty.is_empty());
        assert_eq!(describe(&empty), "0×0");
        assert_eq!(fitted_cells(0, 0, 10, 3, 1), (0, 0));
    }
}
