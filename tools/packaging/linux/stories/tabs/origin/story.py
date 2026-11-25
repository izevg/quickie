"""Observe the browser's passive origin disclosure on real WebKit navigation."""

import time

from PIL import ImageChops

from ..keyboard import control_tab
from ..story import address_visible, text_updated

PAGE = "tabs/origin/fixture.html"
FIRST = (52, 85, 119)
SAME = (96, 66, 117)
CROSS = (61, 111, 112)
QUIET = 0.35


def painted(story, window, color):
    def probe():
        frame = story.picture(window).convert("RGB")
        return frame if frame.getpixel((50 * story.scale, frame.height // 2)) == color else None

    return story.wait(probe, f"page color {color}")


def chip_bounds(image, scale, color):
    center = image.width // 2
    band = image.crop((center - 230 * scale, 9 * scale, center + 230 * scale, 46 * scale))
    idle = band.copy()
    idle.paste(color, (0, 0, band.width, band.height))
    changed = ImageChops.difference(band, idle).convert("L").point(
        lambda value: 255 if value >= 20 else 0,
    )
    bounds = changed.getbbox()
    if bounds is None or bounds[2] - bounds[0] < 100 * scale:
        return None
    # The fixture has no red elements. The insecure status dot distinguishes the native chip
    # from a page repaint or a differently placed browser overlay.
    chip = band.crop(bounds)
    if not any(red > 105 and red > green + 55 and red > blue + 50
               for red, green, blue in chip.getdata()):
        return None
    return bounds


def shown(story, window, color):
    def probe():
        frame = story.picture(window).convert("RGB")
        if frame.getpixel((50 * story.scale, frame.height // 2)) == color:
            return frame if chip_bounds(frame, story.scale, color) else None
        return None

    return story.wait(probe, "the browser's insecure origin chip")


def hidden(story, window, color):
    def probe():
        frame = story.picture(window).convert("RGB")
        page = frame.getpixel((50 * story.scale, frame.height // 2))
        return page == color and chip_bounds(frame, story.scale, color) is None

    story.wait(probe, "origin chip to disappear")


def quiet(story, window, color):
    deadline = time.monotonic() + QUIET

    def probe():
        frame = story.picture(window).convert("RGB")
        assert chip_bounds(frame, story.scale, color) is None, "same origin showed a fresh chip"
        return time.monotonic() >= deadline

    story.wait(probe, "unchanged origin to stay quiet")


def run(story):
    window = story.open(PAGE)
    frame = shown(story, window, FIRST)
    story.save(frame, "origin-chip-http")

    # Click the page precisely underneath the visible chip.
    story.click(window, frame.width // 2, 25 * story.scale)
    story.wait(lambda: story.requests(f"{PAGE}?click=under-chip") == 1, "click behind origin chip")
    assert chip_bounds(story.picture(window).convert("RGB"), story.scale, FIRST), (
        "the click was delivered after the chip had already faded"
    )
    hidden(story, window, FIRST)

    story.click(window, frame.width // 2 - 100 * story.scale, 119 * story.scale)
    painted(story, window, SAME)
    quiet(story, window, SAME)

    story.click(window, frame.width // 2 + 100 * story.scale, 119 * story.scale)
    shown(story, window, CROSS)
    hidden(story, window, CROSS)

    story.key("ctrl+t")
    address = address_visible(story, window)
    story.paste(story.url(PAGE))
    text_updated(story, window, address, 20)
    story.key("Return")
    shown(story, window, FIRST)
    hidden(story, window, FIRST)

    # The first tab committed a rewritten Cyrillic URL. Showing it again reads that URL
    # without asking the server to reload it.
    prior = story.requests(f"{PAGE}?page=russian")
    with control_tab(story):
        pass
    story.move_pointer(window, 4, 4)
    story.save(shown(story, window, CROSS), "origin-chip-russian-path")
    assert story.requests(f"{PAGE}?page=russian") == prior, "switching reloaded the live page"

    story.key("f")
    story.wait(lambda: story.requests(f"{PAGE}?fullscreen=enter") == 1, "page fullscreen")
    image = story.picture(window).convert("RGB")
    assert chip_bounds(image, story.scale, CROSS) is None, "fullscreen kept the tab origin chip"
    story.key("Escape")
    story.wait(lambda: story.requests(f"{PAGE}?fullscreen=leave") == 1, "leave page fullscreen")
    quiet(story, window, CROSS)
