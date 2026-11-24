"""Exercise real WebKit views through GTK's three tab navigation surfaces."""

import time
from pathlib import Path

from PIL import ImageChops

from .keyboard import control_tab

FIXTURE = "tabs/fixture.html"
COLORS = [(52, 85, 119), (96, 66, 117), (130, 116, 72),
          (61, 111, 112), (71, 93, 72), (100, 56, 71)]
REMEMBERED = (32, 112, 96)


def route(page):
    return f"{FIXTURE}?page={page}"


def color(page):
    if page < len(COLORS):
        return COLORS[page]
    return (40 + 5 * page, 65 + 3 * page, 140 - 3 * page)


def painted(story, window, color):
    def matched():
        image = story.picture(window)
        return image if image.getpixel((64 * story.scale, 120 * story.scale)) == color else None

    return story.wait(matched, f"tab page color {color}")


def changed(story, window, idle, label):
    def different():
        image = story.picture(window)
        bounds = ImageChops.difference(image, idle).getbbox()
        if bounds and bounds[2] - bounds[0] > 200 * story.scale:
            return image
        return None

    return story.wait(different, label)


def address_visible(story, window):
    def probe():
        image = story.picture(window)
        y = image.height // 5 + 25 * story.scale
        center = image.getpixel((image.width // 2, y))
        side = image.getpixel((image.width // 8, y))
        return image if max(abs(a - b) for a, b in zip(center, side)) >= 5 else None

    return story.wait(probe, "the new tab's address overlay to take focus")


def text_updated(story, window, before, top_percent):
    scale = story.scale
    center = before.width // 2
    top = before.height * top_percent // 100
    half = (272 if top_percent == 20 else 312) * scale
    area = (center - half, top + 59 * scale, center + half, top + 80 * scale)
    initial = before.crop(area)

    def probe():
        image = story.picture(window)
        contrast = ImageChops.difference(initial, image.crop(area)).convert("L")
        ink = contrast.point(lambda value: 255 if value >= 32 else 0).getbbox()
        return image if ink and ink[2] - ink[0] >= 22 * scale else None

    return story.wait(probe, "typed search text and its native results to appear")


def new_tab(story, window, page):
    path = route(page)
    before = story.requests(path)
    story.key("ctrl+t")
    address = address_visible(story, window)
    story.paste(story.url(path))
    text_updated(story, window, address, 20)
    story.key("Return")
    story.wait(lambda: story.requests(path) > before, f"new tab {page} to navigate")
    return painted(story, window, color(page))


def go_to_tab(story, window):
    painted(story, window, COLORS[0])
    story.click(window, 600 * story.scale, 390 * story.scale)
    painted(story, window, REMEMBERED)
    idle = new_tab(story, window, 1)

    story.key("ctrl+l")
    shown = address_visible(story, window)
    story.key("ctrl+a")
    story.paste("Quickie documentation")
    results = text_updated(story, window, shown, 20)
    story.key("Down")
    story.save(changed(story, window, results, "select an open tab"), "go-to-tab")
    before = story.requests(route(0))
    story.key("Return")
    painted(story, window, REMEMBERED)
    assert story.requests(route(0)) == before, "Go to tab requested the live page again"

    remembered = painted(story, window, REMEMBERED)
    story.key("ctrl+p")
    shown = changed(story, window, remembered, "Go To palette")
    story.paste("Browser scheduling")
    text_updated(story, window, shown, 15)
    story.key("Return")
    painted(story, window, COLORS[1])
    story.key("ctrl+p")
    shown = changed(story, window, idle, "Go To from second tab")
    story.paste("Quickie documentation")
    text_updated(story, window, shown, 15)
    story.key("Return")
    painted(story, window, REMEMBERED)

    story.key("ctrl+w")
    painted(story, window, COLORS[1])
    story.key("ctrl+shift+t")
    return painted(story, window, COLORS[0])


def switcher(story, window, page, scene, actions=()):
    idle = painted(story, window, color(page))
    with control_tab(story, actions):
        if actions:
            # The virtual keyboard sends the first key after the hold has revealed the strip.
            deadline = time.monotonic() + 0.45
            story.wait(lambda: time.monotonic() >= deadline, "switcher keys to reach GTK")
        story.save(tab_panel(story, window, idle, scene), f"tab-switcher-{scene}")


def tab_panel(story, window, idle, scene):
    scale = story.scale
    point = (64 * scale, 120 * scale)
    base = idle.getpixel(point)

    def visible():
        image = story.picture(window)
        dimmed = image.getpixel(point)
        scrim = all(abs(current - value * 0.8) <= 4 for current, value in zip(dimmed, base))
        panel_pixel = image.getpixel((image.width // 3, image.height // 2))
        contrast = max(abs(a - b) for a, b in zip(panel_pixel, dimmed))
        return image if scrim and contrast >= 20 else None

    return story.wait(visible, f"visible native tabs panel for {scene}")


def switcher_scenes(story, window):
    for page in range(2, 6):
        new_tab(story, window, page)
    switcher(story, window, 5, "titles")
    switcher(story, window, 4, "long-titles")
    traversal = ("Tab", "Tab", "Tab", "shift+Tab", "Tab", "Tab", "Tab")
    switcher(story, window, 5, "cycle", traversal)
    painted(story, window, COLORS[5])
    switcher(
        story, window, 5, "filtered",
        ("d", "o", "c", "s", "BackSpace", "Tab", "Tab"),
    )
    painted(story, window, COLORS[0])
    switcher(story, window, 0, "no-match", ("q", "q", "q"))
    painted(story, window, COLORS[0])

    # The selection travels beyond GTK's six-card viewport.
    for page in range(6, 13):
        new_tab(story, window, page)
    switcher(story, window, 12, "many", ("Tab",) * 6)
    return painted(story, window, COLORS[0])


def overview_scenes(story, window, idle):
    story.key("ctrl+shift+backslash")
    overview = tab_panel(story, window, idle, "tab overview")
    story.save(overview, "tab-overview-thumbnails")
    story.paste("Обзор")
    filtered = overview_resized(story, window, overview, shrinking=True)
    story.save(filtered, "tab-overview-filtered")
    story.key("ctrl+a")
    story.paste("no-matching-tab-at-all")
    empty = overview_resized(story, window, filtered, shrinking=True)
    story.save(empty, "tab-overview-no-match")
    story.key("Return")
    story.key("ctrl+a")
    story.paste("Browser scheduling")
    overview_resized(story, window, empty, shrinking=False)
    before = story.requests(route(1))
    story.key("Return")
    painted(story, window, COLORS[1])
    assert story.requests(route(1)) == before, "Tab overview requested a live view again"


def overview_resized(story, window, before, shrinking):
    scale = story.scale

    def height(image):
        x = image.width // 2 - 443 * scale
        scrim = image.getpixel((64 * scale, 120 * scale))
        return sum(
            max(abs(a - b) for a, b in zip(image.getpixel((x, y)), scrim)) >= 12
            for y in range(0, image.height, scale)
        )

    old_height = height(before)

    def resized():
        image = story.picture(window)
        new_height = height(image)
        changed_by = old_height - new_height if shrinking else new_height - old_height
        return image if new_height >= 100 and changed_by >= 60 else None

    return story.wait(resized, "overview to allocate the filtered cards")


def restored_session(story, window):
    story.key("ctrl+w")
    painted(story, window, COLORS[0])
    story.close(window)
    profile = Path(story.environment["XDG_DATA_HOME"]) / "quickie"
    profile.chmod(0o755)
    window = story.open()
    assert profile.stat().st_mode & 0o077 == 0, "The existing browsing profile stayed public"
    idle = painted(story, window, COLORS[0])
    story.key("ctrl+shift+backslash")
    story.save(tab_panel(story, window, idle, "restored overview"), "tab-overview-no-pictures")
    story.key("Escape")
    painted(story, window, COLORS[0])

    # Leave work on the old find controller before switching to a lazily restored tab.
    story.key("ctrl+f")
    story.paste("Sage")
    find_highlight(story, window, idle)
    story.key("ctrl+shift+t")
    fresh = painted(story, window, COLORS[1])
    story.key("ctrl+f")
    story.paste("Cedar")
    story.save(find_highlight(story, window, fresh), "find-after-tab-switch")


def find_highlight(story, window, idle):
    scale = story.scale
    region = (80 * scale, 195 * scale, 245 * scale, 265 * scale)

    def highlighted():
        image = story.picture(window)
        difference = ImageChops.difference(image.crop(region), idle.crop(region)).getbbox()
        return image if difference else None

    return story.wait(highlighted, "Find highlights text in the active native view")


def run(story):
    window = story.open(route(0))
    profile = Path(story.environment["XDG_DATA_HOME"]) / "quickie"
    assert profile.stat().st_mode & 0o077 == 0, "The new browsing profile was created public"
    go_to_tab(story, window)
    idle = switcher_scenes(story, window)
    overview_scenes(story, window, idle)
    restored_session(story, window)
