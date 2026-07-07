#!/usr/bin/env python3
"""Generate a transparent PNG of the FutureOS ASCII banner (Style B warm gradient)."""

from PIL import Image, ImageDraw, ImageFont
import os

# ─── Banner text ──────────────────────────────────────────────────────────
LINES = [
    "  ███████╗██╗   ██╗████████╗██╗   ██╗██████╗ ███████╗     ██████╗ ███████╗",
    "  ██╔════╝██║   ██║╚══██╔══╝██║   ██║██╔══██╗██╔════╝    ██╔═══██╗██╔════╝",
    "  █████╗  ██║   ██║   ██║   ██║   ██║██████╔╝█████╗      ██║   ██║███████╗",
    "  ██╔══╝  ██║   ██║   ██║   ██║   ██║██╔══██╗██╔══╝      ██║   ██║╚════██║",
    "  ██║     ╚██████╔╝   ██║   ╚██████╔╝██║  ██║███████╗    ╚██████╔╝███████║",
    "  ╚═╝      ╚═════╝    ╚═╝    ╚═════╝ ╚═╝  ╚═╝╚══════╝     ╚═════╝ ╚══════╝",
]

# ─── Warm gradient colors (Style B) ───────────────────────────────────────
COLORS = [
    (255, 135, 0),    # orange
    (255, 163, 0),    #
    (255, 191, 0),    # gold
    (255, 223, 0),    # yellow
    (205, 220, 0),    #
    (169, 214, 0),    # lime
]

# ─── Settings ─────────────────────────────────────────────────────────────
FONT_SIZE = 28
PADDING = 48
OUTPUT = "docs/banner.png"

# ─── Find font ────────────────────────────────────────────────────────────
def find_font(size):
    home = os.path.expanduser("~")
    candidates = [
        os.path.join(home, "Library/Fonts/SFMono Bold Nerd Font Complete.otf"),
        os.path.join(home, "Library/Fonts/SFMono Regular Nerd Font Complete.otf"),
        os.path.join(home, "Library/Fonts/JetBrainsMonoNLNerdFont-Bold.ttf"),
        os.path.join(home, "Library/Fonts/JetBrainsMonoNLNerdFont-Regular.ttf"),
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Courier.ttc",
    ]
    for path in candidates:
        if os.path.exists(path):
            try:
                return ImageFont.truetype(path, size)
            except Exception:
                continue
    print("WARNING: no TTF font found, using bitmap default")
    return ImageFont.load_default()

font = find_font(FONT_SIZE)
print(f"Font loaded, size={FONT_SIZE}")

# ─── Measure ──────────────────────────────────────────────────────────────
img_tmp = Image.new("RGBA", (1, 1))
draw_tmp = ImageDraw.Draw(img_tmp)

max_w, max_h = 0, 0
for line in LINES:
    bbox = draw_tmp.textbbox((0, 0), line, font=font)
    max_w = max(max_w, bbox[2] - bbox[0])
    max_h = max(max_h, bbox[3] - bbox[1])

# Add some line spacing
line_h = int(max_h * 1.05)
img_w = max_w + PADDING * 2
img_h = line_h * len(LINES) + PADDING * 2

# ─── Render ───────────────────────────────────────────────────────────────
img = Image.new("RGBA", (img_w, img_h), (0, 0, 0, 0))
draw = ImageDraw.Draw(img)

for i, (line, color) in enumerate(zip(LINES, COLORS)):
    y = PADDING + i * line_h
    draw.text((PADDING, y), line, font=font, fill=color)

img.save(OUTPUT, "PNG")
print(f"Saved: {OUTPUT}  ({img_w}x{img_h})  {os.path.getsize(OUTPUT)} bytes")
