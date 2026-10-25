#!/usr/bin/env python3
"""Rasterize a captured terminal screen (ANSI) into a PNG.

The input is what a real terminal received: the byte stream `future tui` wrote
to its pty, with SGR escapes intact. This module is the *renderer* for that
stream — a terminal emulator's cell model (absolute glyph placement, double-width
CJK, bold/dim/reverse, the 256-colour palette) driving a headless browser for the
actual glyph rasterization. Nothing here invents pixels the app did not emit.

Why not just screenshot a terminal emulator window? The harness is headless and
must be reproducible: a real window needs a display server, a window manager and
a pixel-perfect terminal profile. The trade-off, stated honestly: the glyphs are
drawn by Chrome from the captured cells, not by the terminal that produced them.
What is *not* a trade-off is the content — every character, colour and row comes
from the app's own output.

Ported from `.future/tools/tui-shot/ansi2png.py`, which grew out of a WeChat
illustration job and had two bugs worth keeping fixed: CJK must advance two
columns (or every Chinese line shifts and overlaps), and the cell advance must be
*measured* rather than assumed (`.6em` is wrong for Menlo and made the glyphs look
stretched).

**Glyphs are placed at their cell, never flowed.** The port dropped that and let
the browser lay each row out as text, which is only equivalent when every glyph's
advance equals the cell width. Measured at 14px in headless Chrome with this
font stack: Latin, box drawing, `▸`, `·`, `…` and `█` advance 8.429px (one cell),
but a hanzi advances 14.00px where two cells need 16.858, and full-width
punctuation (`。`) advances 7.175px. So every Chinese row was squeezed and
dragged its tail left, while the border strokes — being exactly one cell — stayed
put: the frame looked aligned at its edges and was wrong inside. On one 120×38
frame that put 28 characters in the wrong cell (including the sidebar's `会话`,
`用量`, `技能` labels and the digits after a Chinese clause), and the sidebar
border on a Chinese row sat 1.6 cells left of every other row.

Every cell is therefore emitted as an absolutely positioned box, and
[`verify_alignment`] re-reads the finished PNG against the grid it came from, so
a capture refuses to write a frame whose glyphs are not in their cells.
"""

from __future__ import annotations

import html
import re
import subprocess
import sys
import unicodedata
from pathlib import Path

CHROME_CANDIDATES = (
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "google-chrome",
    "chromium",
)
FONT = "Menlo, 'SF Mono', 'DejaVu Sans Mono', monospace"
FONT_SIZE = 14
LINE_H = 18
PAD = 16
DEFAULT_FG = "#d8dee9"
DEFAULT_BG = "#1b1d23"


def chrome() -> str:
    for candidate in CHROME_CANDIDATES:
        if Path(candidate).exists():
            return candidate
        found = subprocess.run(["bash", "-lc", f"command -v {candidate}"], capture_output=True, text=True)
        if found.returncode == 0 and found.stdout.strip():
            return found.stdout.strip()
    raise SystemExit("no Chrome/Chromium found for the TUI renderer")


def xterm256() -> list[str]:
    base = [
        "#000000", "#800000", "#008000", "#808000", "#000080", "#800080", "#008080", "#c0c0c0",
        "#808080", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff",
    ]
    out = list(base)
    for r in range(6):
        for g in range(6):
            for b in range(6):
                out.append(f"#{r * 51:02x}{g * 51:02x}{b * 51:02x}")
    for i in range(24):
        v = 8 + i * 10
        out.append(f"#{v:02x}{v:02x}{v:02x}")
    return out


PALETTE = xterm256()


class Cell:
    __slots__ = ("ch", "fg", "bg", "bold", "dim", "reverse", "wide")

    def __init__(self, ch: str = " ") -> None:
        self.ch = ch
        self.fg = None
        self.bg = None
        self.bold = False
        self.dim = False
        self.reverse = False
        self.wide = False


# One pattern for every CSI sequence: `ESC [ params intermediates final`.
# Matching them all matters beyond tidiness — a sequence the parser does not
# recognise has to be *consumed*, because falling through prints its bytes as
# literal text. Measured: `ESC [?25h` (cursor show, which the workbench writes
# with every frame) came out as the characters `[?25h` in the middle of the
# prompt row, and only OCR of the finished PNG caught it.
CSI = re.compile(r"\x1b\[([0-9;?<=>]*)([ -/]*)([@-~])")
# String sequences end at BEL or at ST (ESC \\); both forms occur.
OSC = re.compile(r"\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")


def _apply_sgr(codes: list[int], cell: Cell) -> None:
    """Apply one SGR sequence to the current drawing attributes."""
    j = 0
    while j < len(codes):
        code = codes[j]
        if code == 0:
            cell.fg = cell.bg = None
            cell.bold = cell.dim = cell.reverse = False
        elif code == 1:
            cell.bold = True
        elif code == 2:
            cell.dim = True
        elif code == 7:
            cell.reverse = True
        elif code in (21, 22):
            cell.bold = cell.dim = False
        elif code == 27:
            cell.reverse = False
        elif code == 39:
            cell.fg = None
        elif code == 49:
            cell.bg = None
        elif 30 <= code <= 37:
            cell.fg = PALETTE[code - 30]
        elif 90 <= code <= 97:
            cell.fg = PALETTE[code - 90 + 8]
        elif 40 <= code <= 47:
            cell.bg = PALETTE[code - 40]
        elif 100 <= code <= 107:
            cell.bg = PALETTE[code - 100 + 8]
        elif code in (38, 48):
            value = None
            if j + 1 < len(codes) and codes[j + 1] == 5:
                index = codes[j + 2] if j + 2 < len(codes) else 7
                value = PALETTE[index] if index < len(PALETTE) else None
                j += 2
            elif j + 1 < len(codes) and codes[j + 1] == 2:
                r, g, b = (codes[j + 2 : j + 5] + [0, 0, 0])[:3]
                value = f"#{r:02x}{g:02x}{b:02x}"
                j += 4
            if code == 38:
                cell.fg = value
            else:
                cell.bg = value
        j += 1


def screen_from_stream(text: str, cols: int, rows: int) -> tuple[list[list[Cell]], tuple[int, int] | None]:
    """Replay a captured stream into a `rows` × `cols` cell grid plus the cursor.

    A full-screen TUI addresses every row absolutely (`CSI r;1H`), so replaying
    means honouring those jumps — writing 40 rows once per frame into a growing
    buffer would stack frames on top of each other and the picture would be the
    concatenation of every repaint.

    The cursor returned is where the app left it *and* whether it asked for it
    to be shown: a terminal with an active prompt shows a caret, and a still
    without one reads as a dead window.
    """
    grid = [[Cell() for _ in range(cols)] for _ in range(rows)]
    attrs = Cell()
    row = col = 0
    index = 0
    cursor_shown = False
    while index < len(text):
        if text[index] == "\x1b":
            rest = text[index:]
            match = CSI.match(rest)
            if match:
                params, _intermediate, final = match.group(1), match.group(2), match.group(3)
                codes = [int(c) for c in params.split(";") if c.isdigit()] if params else []
                if final == "m":
                    _apply_sgr(codes or [0], attrs)
                elif final == "H" and len(codes) >= 2:
                    row = codes[0] - 1
                    col = codes[1] - 1
                elif final == "H":
                    row = col = 0
                elif final == "J":
                    # Every frame is redrawn in full, so a wipe is a grid reset.
                    grid = [[Cell() for _ in range(cols)] for _ in range(rows)]
                    row = col = 0
                elif final == "K":
                    # Erase in line: the frames this renderer targets never use
                    # it, but leaving stale cells would be worse than blanking.
                    mode = codes[0] if codes else 0
                    span = range(0, cols) if mode == 2 else (range(col, cols) if mode == 0 else range(0, col + 1))
                    for cx in span:
                        if 0 <= row < rows and 0 <= cx < cols:
                            grid[row][cx] = Cell()
                elif final in "hl" and params.startswith("?"):
                    # Private mode: 25 is cursor visibility; alt screen and mouse
                    # reporting are irrelevant to a still.
                    if "25" in params.split("?", 1)[1].split(";"):
                        cursor_shown = final == "h"
                # Any other final byte (cursor visibility, alt screen, mouse
                # reporting) changes nothing about the cells: consume it.
                index += match.end()
                continue
            match = OSC.match(rest)
            if match:
                index += match.end()
                continue
            index += 1
            continue
        ch = text[index]
        index += 1
        if ch in "\r\n":
            if ch == "\n":
                row += 1
            else:
                col = 0
            continue
        width = 2 if unicodedata.east_asian_width(ch) in ("W", "F") else 1
        if 0 <= row < rows and 0 <= col < cols:
            cell = Cell(ch)
            cell.fg, cell.bg = attrs.fg, attrs.bg
            cell.bold, cell.dim, cell.reverse = attrs.bold, attrs.dim, attrs.reverse
            cell.wide = width == 2
            grid[row][col] = cell
            if width == 2 and col + 1 < cols:
                grid[row][col + 1] = Cell("")
        col += width
    cursor = (row, col) if cursor_shown and 0 <= row < rows and 0 <= col < cols else None
    return grid, cursor


def _cell_style(cell: Cell) -> tuple[str, str | None, bool]:
    """`(fg, bg, bold)` after reverse-video and dim are folded in."""
    fg = cell.fg or DEFAULT_FG
    bg = cell.bg
    if cell.reverse:
        fg, bg = (bg or DEFAULT_BG), (fg or DEFAULT_FG)
    if cell.dim and bg is None:
        fg = "#8b93a1"
    if cell.bold and fg == DEFAULT_FG:
        fg = "#ffffff"
    return fg, bg, cell.bold


def _to_html(grid: list[list[Cell]], cw: float, cols: int, cursor: tuple[int, int] | None = None) -> str:
    """Lay every cell out at the position the grid gives it.

    **Glyphs are placed, never flowed.** A terminal column is a fixed width, and
    a browser's text flow only matches it when every glyph happens to advance by
    exactly that width. In this font stack that is true for Latin, box drawing,
    `·`, `▸`, `…` and `█` — and false for exactly the characters a Chinese
    interface is full of: measured at 14px in headless Chrome, a hanzi advances
    14.00px where two cells need 16.86px, and full-width punctuation (`。`)
    advances 7.17px. Flowing them therefore squeezed every Chinese line and
    dragged everything after it left: 16 wide characters on one row put the
    sidebar border 25px out of place, and a row of Chinese punctuation lost
    nearly a cell per character. The border strokes themselves (`│`, `─`) are
    one cell wide, so the frame looked aligned at its edges while the text
    inside silently was not.

    Absolute placement makes the picture agree with the grid by construction, and
    a wide cell gets a box two cells wide with its glyph centred (a terminal
    fills both columns even when the font's glyph is narrower).
    """
    width = int(cols * cw) + PAD * 2
    height = int(len(grid) * LINE_H) + PAD * 2
    parts = [
        "<!doctype html><meta charset=utf-8>",
        f"<body style='margin:0;background:{DEFAULT_BG}'>",
        f"<div style=\"position:relative;width:{width}px;height:{height}px;background:{DEFAULT_BG}\">",
    ]
    for r, row in enumerate(grid):
        top = PAD + r * LINE_H
        col = 0
        while col < cols:
            cell = row[col]
            if cell.ch == "":
                # The second half of a wide glyph; its box is the wide cell's.
                col += 1
                continue
            span = 2 if cell.wide else 1
            if col + span > cols:
                span = cols - col  # a wide glyph in the last column uses one
            fg, bg, bold = _cell_style(cell)
            blank = cell.ch == " " and cell.bg is None and cell.fg is None and bg is None
            if blank and span == 1:
                col += 1
                continue
            left = PAD + col * cw
            box = cw * span
            style = [
                "position:absolute",
                f"left:{left:.2f}px",
                f"top:{top}px",
                f"width:{box:.2f}px",
                f"height:{LINE_H}px",
                f"line-height:{LINE_H}px",
                f"font-family:{FONT}",
                f"font-size:{FONT_SIZE}px",
            ]
            if bg:
                style.append(f"background:{bg}")
            if cell.wide:
                # A terminal gives a wide glyph the whole two columns: at this
                # font's metrics a hanzi would otherwise advance 14px in a
                # 16.86px box and leave a visible gap, and Chinese text would
                # read as spaced-out. Setting the em to the box width makes the
                # glyph exactly fill its columns. Full-width punctuation is
                # narrower than an em in the fallback font, so it simply centres.
                style.append(f"font-size:{box:.2f}px")
                style.append("text-align:center")
            if not blank:
                style.append(f"color:{fg}")
                style.append(f"font-weight:{'700' if bold else '400'}")
                style.append("white-space:pre")
            parts.append(f"<div style=\"{';'.join(style)}\">{html.escape(cell.ch)}</div>")
            col += span
    # The caret, where the app left it and only if it asked for it to show. A
    # terminal paints it over the cell; a translucent block keeps whatever it
    # covers readable, which is what the eye sees in a live terminal anyway.
    if cursor is not None:
        crow, ccol = cursor
        parts.append(
            f"<div style='position:absolute;left:{PAD + ccol * cw:.2f}px;top:{PAD + crow * LINE_H}px;"
            f"width:{cw:.2f}px;height:{LINE_H}px;background:{DEFAULT_FG};opacity:0.45'></div>"
        )
    parts += ["</div></body>"]
    return "".join(parts)


def cell_width(work: Path) -> float:
    """Measure the glyph advance instead of assuming a fraction of the font size.

    `white-space:nowrap` and a wide window matter: 100 glyphs at ~8.4px is wider
    than the default viewport, so a wrapping probe measures the *line box* and
    reports a nonsense advance (this is how a CJK probe once reported 7.56px per
    character instead of 14).
    """
    probe = work / "_cell.html"
    probe.write_text(
        "<!doctype html><meta charset=utf-8>"
        f"<body style='margin:0'><span id=a style=\"font-family:{FONT};font-size:{FONT_SIZE}px;white-space:nowrap\">"
        + "0" * 100
        + "</span><script>document.title=String(document.getElementById('a').getBoundingClientRect().width/100)</script>",
        encoding="utf-8",
    )
    dom = subprocess.run(
        [chrome(), "--headless=new", "--disable-gpu", "--window-size=4000,800",
         "--virtual-time-budget=2000", "--dump-dom", probe.as_uri()],
        capture_output=True,
        text=True,
    ).stdout
    match = re.search(r"<title>([0-9.]+)</title>", dom)
    return float(match.group(1)) if match else FONT_SIZE * 0.6


def verify_alignment(png: Path, cells: list[list[Cell]], cw: float, scale: float = 2.0) -> list[str]:
    """Check the finished picture against the grid it came from.

    The invariant is the whole point of a terminal renderer: a character's ink
    occupies its own cell and nowhere else. When glyph placement went through the
    browser's text flow instead of absolute positions, characters silently landed
    in the *previous* cell on every row containing Chinese (a hanzi advanced
    14px into a 16.9px cell, `。` 7.2px), and nothing noticed — the PNG looked
    plausible. So every capture now checks its own output: one cell inked where
    the grid says a character, and no character cell left blank.

    Returns a list of human-readable problems (empty means aligned). Stray ink in
    a cell the grid calls blank is *not* a failure: block glyphs in the logo
    overhang their advance box by a pixel or two, which is antialiasing, not a
    misplaced character.
    """
    from PIL import Image

    image = Image.open(png).convert("RGB")
    px = image.load()
    W, H = image.size
    bg = px[5, 5]
    rows = len(cells)
    cols = len(cells[0]) if cells else 0
    problems: list[str] = []
    for r in range(rows):
        y0 = int((PAD + r * LINE_H) * scale)
        y1 = y0 + int(LINE_H * scale)
        for c in range(cols):
            cell = cells[r][c]
            if cell.ch in ("", " ") or (cell.bg is not None or cell.reverse):
                continue
            x0 = int((PAD + c * cw) * scale)
            x1 = int((PAD + (c + 1) * cw) * scale)
            inked = total = 0
            for y in range(max(0, y0), min(H, y1)):
                for x in range(max(0, x0), min(W, x1)):
                    total += 1
                    p = px[x, y]
                    if abs(p[0]-bg[0]) + abs(p[1]-bg[1]) + abs(p[2]-bg[2]) > 24:
                        inked += 1
            if total and inked / total < 0.015:
                problems.append(f"row {r} col {c}: {cell.ch!r} has no ink in its own cell")
    return problems


def render(stream: str, out: Path, cols: int, rows: int, work: Path, scale: float = 2.0) -> tuple[int, int]:
    """Write `out` from a captured `cols` × `rows` stream. Returns the CSS size."""
    cells, cursor = screen_from_stream(stream, cols, rows)
    cw = cell_width(work)
    page = work / "_screen.html"
    page.write_text(_to_html(cells, cw, cols, cursor), encoding="utf-8")
    width = int(cols * cw) + PAD * 2
    height = int(rows * LINE_H) + PAD * 2
    out.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            chrome(),
            "--headless=new",
            "--disable-gpu",
            "--no-sandbox",
            f"--force-device-scale-factor={scale}",
            f"--window-size={width},{height}",
            f"--screenshot={out}",
            page.as_uri(),
        ],
        capture_output=True,
        text=True,
    )
    if not out.exists():
        raise SystemExit(f"renderer produced no file at {out}")
    problems = verify_alignment(out, cells, cw, scale)
    if problems:
        out.unlink(missing_ok=True)
        shown = "\n  ".join(problems[:8])
        raise SystemExit(
            f"error: the rendered frame does not match the cell grid ({len(problems)} cell(s)).\n  "
            + shown
            + "\nThis is a bug in the renderer, not in the capture: glyphs must be"
            " placed at their cell position, never flowed by the browser."
        )
    return width, height


def _self_test() -> int:
    """Check the parser against a stream with known cells (no browser needed).

    Every assertion here is something that silently produced a wrong picture
    once: a frame written row-by-row was stacked instead of replaced, CJK
    advanced one column and shifted the rest of its line, and a reset that
    cleared one row erased the whole grid.
    """
    problems = []

    def check(name, got, want):
        if got != want:
            problems.append(f"{name}: got {got!r}, want {want!r}")

    # Absolute positioning: the second frame must replace the first, not append.
    stream = "\x1b[1;1Hhello\x1b[2;1Hworld" + "\x1b[1;1HHELLO"
    grid, _ = screen_from_stream(stream, 10, 3)
    check("row 0 replaced", "".join(c.ch for c in grid[0]).rstrip(), "HELLO")
    check("row 1 kept", "".join(c.ch for c in grid[1]).rstrip(), "world")

    # SGR colour and bold survive into the cell.
    grid, _ = screen_from_stream("\x1b[31;1mX", 4, 1)
    check("fg", grid[0][0].fg, PALETTE[1])
    check("bold", grid[0][0].bold, True)

    # Reverse swaps at paint time; the cell records the flag.
    grid, _ = screen_from_stream("\x1b[7mR\x1b[27mN", 4, 1)
    check("reverse on", grid[0][0].reverse, True)
    check("reverse off", grid[0][1].reverse, False)

    # CJK takes two columns; the cell after it starts at 3.
    grid, _ = screen_from_stream("中文x", 6, 1)
    check("cjk lead", grid[0][0].ch, "中")
    check("cjk wide", grid[0][0].wide, True)
    check("cjk tail", grid[0][1].ch, "")
    check("after cjk", grid[0][2].ch, "文")
    check("latin after cjk", grid[0][4].ch, "x")

    # A clear (J) empties the grid; a home (H) does not.
    grid, _ = screen_from_stream("\x1b[1;1Habc\x1b[2J", 4, 2)
    check("clear wipes", "".join(c.ch for c in grid[0]).strip(), "")
    grid, _ = screen_from_stream("\x1b[1;1Habc\x1b[H", 4, 1)
    check("home keeps", "".join(c.ch for c in grid[0]).rstrip(), "abc")

    # Unrecognised CSI is consumed, never printed. `[?25h` leaking as text into
    # the prompt row is what OCR of a finished screenshot caught.
    grid, _ = screen_from_stream("\x1b[?25lab\x1b[?25hcd\x1b[?1006h", 8, 1)
    check("private modes invisible", "".join(c.ch for c in grid[0]).rstrip(), "abcd")

    # OSC (window title) is consumed in both terminators.
    grid, _ = screen_from_stream("\x1b]0;title\x07ok\x1b]2;other\x1b\\!", 8, 1)
    check("osc invisible", "".join(c.ch for c in grid[0]).rstrip(), "ok!")

    # The cursor is reported only while the app has asked for it to show.
    _, cursor = screen_from_stream("\x1b[?25lab\x1b[?25H\x1b[3;4H", 8, 4)
    check("cursor hidden until shown", cursor, None)
    _, cursor = screen_from_stream("\x1b[?25lab\x1b[?25h\x1b[3;4Hx", 8, 4)
    check("cursor position", cursor, (2, 4))

    # Placement is absolute: a cell's div carries its own left/width, so the
    # browser never flows one glyph into another's cell. This is the regression
    # guard for the bug that made every Chinese row shift (see _to_html).
    grid = [[Cell("A"), Cell("中"), Cell(" ")], [Cell("B"), Cell(" "), Cell(" ")]]
    grid[0][1].wide = True
    markup = _to_html(grid, 10.0, 3)
    check("narrow cell at its column", "left:16.00px" in markup, True)
    check("second row starts on its own line", "top:34px" in markup, True)
    check("wide cell at its own column", "left:26.00px" in markup, True)
    check("wide cell is two cells wide", "width:20.00px" in markup, True)
    # A wide cell sets its em to the box width, so the glyph fills both columns
    # instead of sitting narrow in a wide box with a gap after it.
    check("wide cell fills its box", "font-size:20.00px" in markup, True)

    if problems:
        for problem in problems:
            print(f"FAIL {problem}")
        return 1
    print("tui_terminal self-test: ok")
    return 0


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        raise SystemExit(_self_test())
    source, target, columns, lines = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
    work = Path("/tmp/future-tui-shot")
    work.mkdir(parents=True, exist_ok=True)
    size = render(Path(source).read_text(encoding="utf-8"), Path(target), columns, lines, work)
    print(f"{target}: {size[0]}x{size[1]} css px")
