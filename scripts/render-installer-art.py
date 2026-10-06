#!/usr/bin/env python3
"""Render the macOS DMG background and the Windows installer bitmaps.

Icon positions match src-tauri/tauri.conf.json. Finder places the real
application icon and the Applications alias; this background is only the
frame, the arrow, and the caption.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "src-tauri" / "icons"
WINDOWS = ROOT / "src-tauri" / "windows"

WINDOW = (660, 400)
APP = (180, 170)
APPLICATIONS = (480, 170)

FONT_SEMI = "/usr/share/fonts/truetype/macos/Inter-SemiBold.ttf"
FONT_REGULAR = "/usr/share/fonts/truetype/macos/Inter-Regular.ttf"


def font(path: str, size: int) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(path, size)


def lerp(a: int, b: int, t: float) -> int:
    return int(a + (b - a) * t)


def gradient(size: tuple[int, int]) -> Image.Image:
    width, height = size
    image = Image.new("RGB", size)
    pixels = image.load()
    top = (8, 12, 22)
    mid = (16, 28, 52)
    bottom = (22, 58, 122)
    for y in range(height):
        t = y / max(height - 1, 1)
        if t < 0.55:
            u = t / 0.55
            color = tuple(lerp(top[i], mid[i], u) for i in range(3))
        else:
            u = (t - 0.55) / 0.45
            color = tuple(lerp(mid[i], bottom[i], u) for i in range(3))
        for x in range(width):
            pixels[x, y] = color
    return image


def rounded_rect(draw: ImageDraw.ImageDraw, box, radius, fill) -> None:
    draw.rounded_rectangle(box, radius=radius, fill=fill)


def arrow(draw: ImageDraw.ImageDraw, scale: int) -> None:
    left, right = APP[0] * scale, APPLICATIONS[0] * scale
    y = APP[1] * scale
    gap_start = left + 78 * scale
    gap_end = right - 78 * scale
    shaft_end = gap_start + int((gap_end - gap_start) * 0.62)
    thickness = 7 * scale
    color = (236, 242, 252)
    draw.rectangle(
        [gap_start, y - thickness // 2, shaft_end, y + thickness // 2],
        fill=color,
    )
    head = 22 * scale
    draw.polygon(
        [
            (shaft_end - 2 * scale, y - head),
            (shaft_end + int(head * 1.35), y),
            (shaft_end - 2 * scale, y + head),
        ],
        fill=color,
    )


def dmg(scale: int) -> Image.Image:
    size = (WINDOW[0] * scale, WINDOW[1] * scale)
    image = gradient(size)
    draw = ImageDraw.Draw(image)
    rounded_rect(
        draw,
        (36 * scale, 78 * scale, (WINDOW[0] - 36) * scale, 292 * scale),
        28 * scale,
        (12, 18, 34),
    )
    arrow(draw, scale)
    title = "Unit Agent"
    title_font = font(FONT_SEMI, 28 * scale)
    caption_font = font(FONT_REGULAR, 15 * scale)
    title_box = draw.textbbox((0, 0), title, font=title_font)
    title_w = title_box[2] - title_box[0]
    draw.text(
        ((size[0] - title_w) / 2, 28 * scale),
        title,
        font=title_font,
        fill=(255, 255, 255),
    )
    caption = "Drag Unit Agent to Applications, then eject this disk."
    caption_box = draw.textbbox((0, 0), caption, font=caption_font)
    caption_w = caption_box[2] - caption_box[0]
    draw.text(
        ((size[0] - caption_w) / 2, 332 * scale),
        caption,
        font=caption_font,
        fill=(198, 210, 230),
    )
    return image


def header() -> Image.Image:
    image = gradient((150, 57))
    draw = ImageDraw.Draw(image)
    draw.polygon([(18, 16), (34, 28), (18, 40)], fill=(255, 255, 255))
    draw.text((44, 16), "Unit Agent", font=font(FONT_SEMI, 16), fill=(255, 255, 255))
    return image


def sidebar() -> Image.Image:
    image = gradient((164, 314))
    draw = ImageDraw.Draw(image)
    draw.polygon([(46, 108), (118, 154), (46, 200)], fill=(255, 255, 255))
    label = "Unit Agent"
    label_font = font(FONT_SEMI, 16)
    box = draw.textbbox((0, 0), label, font=label_font)
    width = box[2] - box[0]
    draw.text(((164 - width) / 2, 248), label, font=label_font, fill=(255, 255, 255))
    return image


def main() -> None:
    ICONS.mkdir(parents=True, exist_ok=True)
    WINDOWS.mkdir(parents=True, exist_ok=True)
    dmg(1).save(ICONS / "dmg-background.png", dpi=(72, 72))
    dmg(2).save(ICONS / "dmg-background@2x.png", dpi=(144, 144))
    header().convert("RGB").save(WINDOWS / "nsis-header.bmp")
    sidebar().convert("RGB").save(WINDOWS / "nsis-sidebar.bmp")


if __name__ == "__main__":
    main()
