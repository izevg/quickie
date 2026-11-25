"""Drive the native GTK address overlay, command palette, and history."""

import time
from typing import NamedTuple

from PIL import Image, ImageChops

FIXTURE = "palette/fixture.html"
READY = {
    "home": (95, 226, 176),
    "webkit": (246, 185, 94),
    "cyrillic": (125, 169, 251),
    "long": (236, 139, 186),
}


class Panel(NamedTuple):
    image: Image.Image
    bounds: tuple[int, int, int, int]


def loaded(story, window, page, size=None):
    def rendered():
        image = story.picture(window)
        if size is not None and image.size != tuple(side * story.scale for side in size):
            return None
        marker = (image.width - 18 * story.scale, image.height - 18 * story.scale)
        return image if image.getpixel(marker) == READY[page] else None

    return story.wait(rendered, f"the {page} fixture route to paint")


def difference(before, after):
    if before.size != after.size:
        return None
    return ImageChops.difference(before, after).getbbox()


def panel(
    story, window, idle, name, width=460, shorter_than=None, taller_than=None,
    different_results_from=None, capture=True,
):
    def visible():
        image = story.picture(window)
        bounds = difference(idle, image)
        if bounds is None or bounds[2] - bounds[0] < width * story.scale:
            return None
        top_strip = (0, 0, image.width, 4 * story.scale)
        outside = ImageChops.difference(idle.crop(top_strip), image.crop(top_strip))
        # The shadow can change this strip in a narrow window, but the page stays visible.
        if any(maximum > 32 for _, maximum in outside.getextrema()):
            return None
        if shorter_than is not None and bounds[3] >= shorter_than.bounds[3]:
            return None
        if taller_than is not None and bounds[3] <= taller_than.bounds[3]:
            return None
        if different_results_from is not None and not results_changed(
            different_results_from, image, story.scale
        ):
            return None
        return Panel(image, bounds)

    rendered = story.wait(visible, f"native panel {name} to paint")
    if capture:
        story.save(rendered.image, name)
    return rendered


def results_changed(previous, image, scale):
    area = results_area(previous.bounds, scale)
    # Ignore subtle backdrop changes and require a span of high-contrast content to repaint.
    changes = contrast_changes(previous.image.crop(area), image.crop(area), 32)
    return changes is not None and changes[2] - changes[0] >= 24 * scale


def contrast_changes(before, after, threshold):
    delta = ImageChops.difference(before, after).convert("L")
    return delta.point(lambda level: 255 if level >= threshold else 0).getbbox()


def results_area(bounds, scale):
    left, top, right, bottom = bounds
    # The bounds include the shadow. Start below the entry and its focus border.
    return left + 24 * scale, top + 128 * scale, right - 24 * scale, bottom


def replace(story, text):
    story.key("ctrl+a")
    story.paste(text)


def dismiss(story, window, idle):
    story.key("Escape")
    story.wait(
        lambda: equal(idle, story.picture(window)),
        "Escape to restore the page",
    )


def equal(before, after):
    return before.size == after.size and difference(before, after) is None


def submit(story, window, page, path):
    before = story.requests(path)
    story.key("Return")
    story.wait(lambda: story.requests(path) > before, f"Enter to request {page}")
    return loaded(story, window, page)


def address_scenes(story, window):
    idle = loaded(story, window, "home")
    story.save(idle, "page-frame-edge-to-edge")
    story.key("ctrl+l")
    current = panel(story, window, idle, "address-current-url")
    webkit = f"{FIXTURE}?page=webkit"
    replace(story, story.url(webkit))
    panel(story, window, idle, "address-url-typed", different_results_from=current)
    idle = submit(story, window, "webkit", webkit)

    long_route = f"{FIXTURE}?page=long&path=" + "a-very-long-path-" * 9
    story.key("ctrl+l")
    current = panel(story, window, idle, "address before long edit", capture=False)
    replace(story, story.url(long_route))
    panel(story, window, idle, "address-long-url", different_results_from=current)
    idle = submit(story, window, "long", long_route)

    cyrillic = f"{FIXTURE}?page=cyrillic"
    story.key("ctrl+l")
    current = panel(story, window, idle, "address before visiting Cyrillic", capture=False)
    replace(story, story.url(cyrillic))
    panel(story, window, idle, "typed Cyrillic route", different_results_from=current, capture=False)
    return submit(story, window, "cyrillic", cyrillic)


def command_scenes(story, window, idle):
    story.key("ctrl+shift+p")
    commands = panel(story, window, idle, "palette-commands")
    replace(story, "rel")
    filtered = panel(
        story, window, idle, "palette-highlights", shorter_than=commands
    )
    replace(story, "no-command-matches-xzy")
    panel(story, window, idle, "palette-empty", shorter_than=filtered)
    replace(story, "rel")

    scale = story.scale
    rows = results_area(filtered.bounds, scale)
    filtered_rows = filtered.image.crop(rows)

    def filter_restored():
        image = story.picture(window)
        # Repainting a translucent panel can vary the backdrop without changing its rows.
        return image if contrast_changes(filtered_rows, image.crop(rows), 16) is None else None

    initial = story.wait(filter_restored, "reload results to return").crop(rows)
    story.key("Down")

    def selection_changed():
        image = story.picture(window)
        changes = contrast_changes(initial, image.crop(rows), 20)
        return image if (
            changes is not None and changes[2] - changes[0] >= 24 * scale
            and changes[3] - changes[1] >= 24 * scale
        ) else None

    selected = story.wait(selection_changed, "Down to select another command")
    story.save(selected, "palette-run-command")
    submit(story, window, "cyrillic", f"{FIXTURE}?page=cyrillic")

    def restored():
        image = story.picture(window)
        return image if equal(idle, image) else None

    story.save(story.wait(restored, "the executed command to close the palette"), "palette-reload-result")


def history_scenes(story, window, idle):
    story.key("ctrl+p")
    history = panel(story, window, idle, "palette-go-to")
    replace(story, "browser scheduling")
    long_title = panel(story, window, idle, "palette-long-titles", shorter_than=history)
    replace(story, "Обзор")
    matched = panel(
        story, window, idle, "palette-cyrillic", shorter_than=history,
        different_results_from=long_title,
    )
    replace(story, "unmatched-local-history-query")
    unmatched = panel(
        story, window, idle, "palette-history-empty", shorter_than=matched
    )
    cyrillic = f"{FIXTURE}?page=cyrillic"
    count = story.requests(cyrillic)
    story.key("Return")
    deadline = time.monotonic() + 0.5
    story.wait(lambda: time.monotonic() >= deadline, "the empty history action to settle")
    after_enter = loaded(story, window, "cyrillic")
    assert story.requests(cyrillic) == count, "An empty history result reloaded the page"
    assert (
        difference(idle, after_enter) == unmatched.bounds
    ), "Empty history Enter changed the panel"
    dismiss(story, window, idle)

    story.key("ctrl+l")
    current = panel(story, window, idle, "address before search", capture=False)
    replace(story, "WebKit")
    search = panel(story, window, idle, "address-history-match", taller_than=current)
    replace(story, "Обзор")
    panel(story, window, idle, "address-cyrillic", different_results_from=search)
    dismiss(story, window, idle)

    story.key("ctrl+l")
    address = panel(story, window, idle, "address before completion", capture=False)
    replace(story, story.url(f"{FIXTURE}?page=web"))
    prefix = panel(story, window, idle, "address-url-prefix", taller_than=address)
    story.key("Tab")
    panel(story, window, idle, "address-url-completed", shorter_than=prefix)
    webkit = submit(story, window, "webkit", f"{FIXTURE}?page=webkit")
    visit_cyrillic(story, window, webkit)


def visit_cyrillic(story, window, idle):
    story.key("ctrl+p")
    history = panel(story, window, idle, "history before search", capture=False)
    replace(story, "Обзор")
    panel(
        story, window, idle, "filtered history before Enter",
        shorter_than=history, capture=False,
    )
    submit(story, window, "cyrillic", f"{FIXTURE}?page=cyrillic")


def narrow_scenes(story, window):
    def resized(width, height):
        size = (width, height)
        story.resize(window, *size)
        loaded(story, window, "cyrillic", size=size)
        # Wayland software cursors can enter or leave the crop when a window moves.
        story.move_pointer(window, 4, 4)
        return loaded(story, window, "cyrillic", size=size)

    idle = resized(560, 520)
    story.key("ctrl+shift+p")
    panel(story, window, idle, "palette-narrow", width=360)
    dismiss(story, window, idle)
    idle = resized(360, 520)
    story.key("ctrl+l")
    panel(story, window, idle, "address-narrow", width=250)
    dismiss(story, window, idle)
    idle = resized(1200, 780)
    story.key("ctrl+shift+p")
    panel(story, window, idle, "palette-before-outside-click")
    story.click(window, 32 * story.scale, 670 * story.scale)
    story.move_pointer(window, 4, 4)
    story.wait(
        lambda: equal(idle, story.picture(window)),
        "an outside click to dismiss the palette",
    )


def run(story):
    window = story.open(FIXTURE)
    story.move_pointer(window, 4, 4)
    idle = address_scenes(story, window)
    command_scenes(story, window, idle)
    history_scenes(story, window, idle)
    narrow_scenes(story, window)
