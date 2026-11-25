"""Exercise the WebKitGTK script dialogs through the native browser UI."""

from urllib.parse import urlencode

from PIL import ImageChops

FIXTURE = "dialogs/fixture.html"
READY = (95, 226, 176)
NAME = "Ада Lovelace"


def page_marker(image, scale):
    return image.getpixel((image.width - 18 * scale, image.height - 18 * scale))


def panel_bounds(image, baseline, scale):
    if image.size != baseline.size:
        return None
    marker = page_marker(image, scale)
    if any(abs(2 * channel - initial) > 2 for channel, initial in zip(marker, READY)):
        return None
    # The native dialog dims the page by half. Require a painted widget inside that backdrop.
    center = (image.width // 6, image.height // 5, image.width * 5 // 6, image.height * 4 // 5)
    dimmed = baseline.crop(center).point(lambda channel: channel // 2)
    changed = ImageChops.difference(dimmed, image.crop(center))
    bounds = changed.convert("L").point(lambda difference: 255 if difference >= 16 else 0).getbbox()
    if bounds and bounds[2] - bounds[0] >= 120 * scale and bounds[3] - bounds[1] >= 65 * scale:
        return tuple(offset + edge for offset, edge in zip(center[:2] * 2, bounds))
    return None


def open_scene(story, window, baseline, key, scene):
    story.key(key)

    def painted():
        image = story.picture(window)
        return image if panel_bounds(image, baseline, story.scale) is not None else None

    return story.wait(painted, f"painted native {scene} dialog")


def panel_height(image, baseline, scale):
    _, top, _, bottom = panel_bounds(image, baseline, scale)
    return bottom - top


def answer(story, window, scene, result, key):
    path = FIXTURE + "?" + urlencode({"scene": scene, "answer": result})
    previous = story.requests(path)
    story.key(key)
    story.wait(lambda: story.requests(path) > previous, f"page receives {scene} response")

    def restored():
        image = story.picture(window)
        return image if page_marker(image, story.scale) == READY else None

    return story.wait(restored, f"{scene} dialog closes")


def prompt_frame(story, window, baseline, empty):
    bounds = panel_bounds(empty, baseline, story.scale)
    story.paste(NAME)

    def typed():
        image = story.picture(window)
        if panel_bounds(image, baseline, story.scale) is None:
            return None
        changes = ImageChops.difference(empty.crop(bounds), image.crop(bounds)).getbbox()
        # A caret can blink even before the entry receives the clipboard text.
        return image if changes and changes[2] - changes[0] >= 40 * story.scale else None

    return story.wait(typed, "prompt entry displays the pasted Cyrillic name")


def run(story):
    window = story.open(FIXTURE)

    def ready():
        image = story.picture(window)
        return image if page_marker(image, story.scale) == READY else None

    baseline = story.wait(ready, "WebKit paints the dialogs fixture")
    alert = open_scene(story, window, baseline, "F2", "alert")
    baseline = answer(story, window, "alert", "dismissed", "Return")
    story.save(alert, "dialog-sheet-alert")

    confirm = open_scene(story, window, baseline, "F3", "confirmation")
    short_height = panel_height(confirm, baseline, story.scale)
    baseline = answer(story, window, "confirm-block", "false", "Escape")
    open_scene(story, window, baseline, "F3", "confirmation")
    baseline = answer(story, window, "confirm-block", "true", "Return")
    story.save(confirm, "dialog-sheet-confirm-block")

    empty = open_scene(story, window, baseline, "F4", "prompt")
    prompt = prompt_frame(story, window, baseline, empty)
    baseline = answer(story, window, "prompt", NAME, "Return")
    story.save(prompt, "dialog-sheet-prompt")

    long_message = open_scene(story, window, baseline, "F6", "long message")
    assert panel_height(long_message, baseline, story.scale) >= short_height + 24 * story.scale, (
        "The long message must wrap across more lines than a short confirmation"
    )
    answer(story, window, "long-message", "false", "Escape")
    story.save(long_message, "dialog-sheet-long-message")
