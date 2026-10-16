"""Offline regression tests for the upright check's judge.

`capture.py check-upright-mobile` renders a mobile scenario with
`upright-probe.js`, which puts the transcript's own text beside an unflipped copy
of it, and then decides from the pixels whether the capture reads the right way
up. That decision is what these tests pin down: they build the probe's two tiles
by hand (no browser, no dev server, no Chrome) and check that

  * an upright in-place tile beats its own mirror,
  * a mirrored in-place tile loses to its own mirror (the defect the check
    exists to catch — if this direction ever flips, the check would bless the
    very screenshots it is meant to reject), and
  * a tile with no vertical structure at all is reported as inconclusive rather
    than as a pass.

The tiles are drawn, not captured: a couple of asymmetric strokes on a white
field, which is all the judge needs. See docs/guide/screenshots.md.
"""
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "screenshots" / "capture.py"

spec = importlib.util.spec_from_file_location("capture", SCRIPT)
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)

VIEWPORT = {"w": 390, "h": 844, "mobile": True}
SCALE = 2
OUTLINE = 8  # image px; capture.py insets the outline box before comparing


def text_tile(width: int = 200, height: int = 40, mirrored: bool = False):
    """A tile that reads as a line of text: ink high on the left, low on the right."""
    from PIL import Image, ImageDraw

    tile = Image.new("L", (width, height), 255)
    draw = ImageDraw.Draw(tile)
    for index, (x, y, w, h) in enumerate(((0, 2, 10, 30), (14, 4, 8, 8), (14, 26, 8, 8),
                                          (30, 6, 6, 28), (48, 2, 12, 10), (48, 22, 12, 6),
                                          (70, 8, 10, 24), (90, 18, 14, 6), (110, 0, 8, 22))):
        draw.rectangle([x, y, x + w, y + h], fill=20 + index * 8)
    if mirrored:
        tile = tile.transpose(Image.FLIP_TOP_BOTTOM)
    return tile


def probe_image(in_place, control):
    """A stand-in for what the probe's screenshot looks like: two outlined tiles."""
    from PIL import Image

    image = Image.new("RGB", (VIEWPORT["w"] * SCALE, VIEWPORT["h"] * SCALE), (255, 255, 255))
    for tile, position, colour in ((in_place, (40, 200), capture.UPRIGHT_IN_PLACE),
                                   (control, (12, 1000), capture.UPRIGHT_CONTROL)):
        frame = Image.new("RGB", (tile.width + 2 * OUTLINE, tile.height + 2 * OUTLINE), colour)
        frame.paste(Image.merge("RGB", (tile, tile, tile)), (OUTLINE, OUTLINE))
        image.paste(frame, position)
    return image


class UprightJudge(unittest.TestCase):
    def judge(self, in_place, control) -> tuple:
        """Run the checker's judge over a synthetic probe screenshot."""
        with tempfile.TemporaryDirectory(prefix="future-upright-test-") as directory:
            path = Path(directory) / "probe.png"
            probe_image(in_place, control).save(path)
            stream = io.StringIO()
            with mock.patch("sys.stdout", stream):
                code = capture.report_upright(path, VIEWPORT)
        return code, stream.getvalue()

    def test_upright_tile_is_accepted(self):
        code, report = self.judge(text_tile(), text_tile())
        self.assertEqual(code, 0, report)
        self.assertIn("verdict: upright", report)

    def test_mirrored_tile_is_rejected(self):
        code, report = self.judge(text_tile(mirrored=True), text_tile())
        self.assertEqual(code, 1, report)
        self.assertIn("verdict: MIRRORED", report)

    def test_a_tile_without_structure_is_inconclusive(self):
        from PIL import Image

        flat = Image.new("L", (200, 40), 255)
        code, report = self.judge(flat, flat)
        self.assertEqual(code, 2, report)
        self.assertIn("inconclusive", report)

    def test_a_missing_outline_is_an_error(self):
        with tempfile.TemporaryDirectory(prefix="future-upright-test-") as directory:
            path = Path(directory) / "plain.png"
            from PIL import Image

            Image.new("RGB", (VIEWPORT["w"] * SCALE, VIEWPORT["h"] * SCALE), (255, 255, 255)).save(path)
            stream, errors = io.StringIO(), io.StringIO()
            with mock.patch("sys.stdout", stream), mock.patch("sys.stderr", errors):
                code = capture.report_upright(path, VIEWPORT)
        self.assertEqual(code, 1, stream.getvalue())
        self.assertIn("no in-place tile", errors.getvalue())


class Correlation(unittest.TestCase):
    def grid(self, rows: int = 20, columns: int = 12) -> list:
        return [[float((x * 7 + y * 3) % 11) for x in range(columns)] for y in range(rows)]

    def test_identical_grids_correlate_fully(self):
        rows = self.grid()
        self.assertAlmostEqual(capture._best_correlation(rows, rows), 1.0, places=6)

    def test_a_vertical_mirror_of_an_asymmetric_grid_does_not(self):
        rows = self.grid()
        self.assertLess(capture._best_correlation(rows, list(reversed(rows))), 0.5)

    def test_alignment_tolerance_absorbs_a_one_pixel_offset(self):
        rows = self.grid()
        shifted = [[0.0] * len(rows[0])] + rows[:-1]
        self.assertGreater(capture._best_correlation(rows, shifted), 0.99)


if __name__ == "__main__":
    unittest.main(verbosity=2)
