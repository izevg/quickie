"""Drive native Link Hints against a live page and assert where keyboard picks take effect."""

from functools import partial
from threading import Event

from PIL import ImageChops

from tabs.keyboard import control_tab

FIXTURE = "tabs/hints/fixture.html"
ENTRY = (34, 56, 75)
GREEN = (36, 94, 73)
PURPLE = (83, 59, 113)
BUTTON_ACTIVATED = (96, 225, 173)
FIELD_FOCUSED = (196, 135, 232)
TARGET_TOPS = (92, 136, 180, 224, 268, 312)


def painted(story, window, color, x=12, y=50):
    def match():
        image = story.picture(window)
        return image if image.getpixel((x * story.scale, y * story.scale)) == color else None

    return story.wait(match, f"the hint fixture color {color}")


def visible_labels(story, image, idle):
    return sum(
        label_at(story.scale, image, idle, left, top)
        for top in TARGET_TOPS for left in (60, 360)
    )


def label_at(scale, image, idle, left, top):
    # Sample inside the badge, away from the page control's focus outline.
    box = tuple(coordinate * scale for coordinate in (left, top, left + 12, top + 11))
    red, green, blue = ImageChops.difference(idle.crop(box), image.crop(box)).split()
    difference = ImageChops.lighter(ImageChops.lighter(red, green), blue)
    return sum(difference.histogram()[32:]) >= 53 * scale ** 2


def hints(story, window, idle, count):
    def match():
        current = story.picture(window)
        return current if visible_labels(story, current, idle) == count else None

    return story.wait(match, f"exactly {count} native link labels")


def pick(story, window, idle, code):
    story.key("alt+f")
    hints(story, window, idle, 12)
    story.key(code)


def resize_dismisses(story, window):
    idle = painted(story, window, ENTRY)
    story.key("alt+f")
    hints(story, window, idle, 12)

    story.resize(window, 720, 560)

    def resized_and_clear():
        image = story.picture(window)
        resized = image.width == 720 * story.scale
        return image if resized and visible_labels(story, image, idle) == 0 else None

    story.wait(resized_and_clear, "a resized viewport has no stale hints")


def collecting_ignores_typing(story, window):
    idle = painted(story, window, ENTRY)
    story.click(window, 400 * story.scale, 108 * story.scale)
    painted(story, window, FIELD_FOCUSED, 72, 442)
    waiting = Event()
    release = Event()
    factory = story.server.RequestHandlerClass

    class PausedFixture(factory.func):
        def do_GET(self):
            if self.path == f"/{FIXTURE}?waiting_for_hints":
                waiting.set()
                if not release.wait(15):
                    self.send_error(504)
                    return
            super().do_GET()

    story.server.RequestHandlerClass = partial(PausedFixture, directory=factory.keywords["directory"])
    try:
        story.key("F8")
        story.wait(waiting.is_set, "the WebKit page is waiting for the test server")
        story.key("alt+f", "a", "BackSpace")
    finally:
        release.set()
        story.server.RequestHandlerClass = factory

    hints(story, window, idle, 12)
    story.key("Escape", "F9")
    story.wait(
        lambda: story.requests(f"{FIXTURE}?keys=unchanged") or story.requests(f"{FIXTURE}?keys=leaked"),
        "the focused WebKit input reports its key events",
    )
    assert story.requests(f"{FIXTURE}?keys=unchanged") == 1, "hint keys reached the focused page input"


def run(story):
    window = story.open(FIXTURE)
    idle = painted(story, window, ENTRY)

    story.key("alt+f")
    labels = hints(story, window, idle, 12)
    assert not label_at(story.scale, labels, idle, 60, 372), "a disabled control has no hint"
    story.save(labels, "link-hints-labels")
    story.key("q", "l")
    narrowed = hints(story, window, idle, 4)
    assert all(
        label_at(story.scale, narrowed, idle, left, top)
        for top in TARGET_TOPS[-2:] for left in (60, 360)
    ), "the four labels starting with L stay visible"
    story.save(narrowed, "link-hints-narrowed")
    story.key("BackSpace")
    hints(story, window, idle, 12)
    story.key("l", "a")
    green = painted(story, window, GREEN)
    story.save(green, "link-hints-followed")
    assert story.requests(f"{FIXTURE}?destination=green") == 1

    story.key("alt+Left")
    idle = painted(story, window, ENTRY)
    story.key("shift+alt+f")
    story.save(hints(story, window, idle, 12), "link-hints-background")
    story.key("d")
    story.wait(lambda: story.requests(f"{FIXTURE}?destination=green") == 2, "background Tab load")
    hints(story, window, idle, 0)
    painted(story, window, ENTRY)
    with control_tab(story, hold_ms=180):
        pass
    painted(story, window, GREEN)
    story.key("ctrl+w")

    idle = painted(story, window, ENTRY)
    pick(story, window, idle, "f")
    painted(story, window, PURPLE)
    assert story.requests(f"{FIXTURE}?destination=purple") == 1
    story.key("ctrl+w")

    idle = painted(story, window, ENTRY)
    pick(story, window, idle, "a")
    story.wait(lambda: story.requests(f"{FIXTURE}?activated=control") == 1, "hint button click")
    idle = painted(story, window, BUTTON_ACTIVATED, 72, 442)
    pick(story, window, idle, "s")
    painted(story, window, FIELD_FOCUSED, 72, 442)
    story.key("x")
    story.wait(lambda: story.requests(f"{FIXTURE}?typed=field") == 1, "hint input focus")

    idle = painted(story, window, ENTRY)
    story.key("alt+f")
    hints(story, window, idle, 12)
    story.key("Escape")
    hints(story, window, idle, 0)
    story.key("alt+f")
    hints(story, window, idle, 12)
    story.key("alt+f")
    hints(story, window, idle, 0)
    story.key("alt+f")
    hints(story, window, idle, 12)
    story.click(window, 15 * story.scale, 70 * story.scale)
    hints(story, window, idle, 0)

    idle = painted(story, window, ENTRY)
    story.key("alt+f")
    hints(story, window, idle, 12)
    count = story.requests(FIXTURE)
    story.key("ctrl+r")
    story.wait(lambda: story.requests(FIXTURE) > count, "reload")
    painted(story, window, ENTRY)
    hints(story, window, idle, 0)

    collecting_ignores_typing(story, window)
    resize_dismisses(story, window)
